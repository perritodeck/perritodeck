import Combine
import Foundation
import Network

/// A PerritoDeck host advertising itself over Bonjour.
public struct FoundHost: Equatable, Identifiable, Sendable {
    /// The Bonjour service instance name ("PerritoDeck on dev-laptop").
    public var serviceName: String
    /// The host's display name (TXT `n`), or the service name.
    public var name: String
    public var address: String
    public var port: Int
    /// The host's certificate fingerprint (TXT `id`), if advertised.
    public var hostId: String?

    public var id: String { serviceName }

    public init(serviceName: String, name: String, address: String, port: Int, hostId: String?) {
        self.serviceName = serviceName
        self.name = name
        self.address = address
        self.port = port
        self.hostId = hostId
    }
}

/// Finds hosts advertising `_perritodeck._tcp` with NWBrowser. Mirrors Android's Discovery.
///
/// NWBrowser reports service names and TXT records but not addresses. URLSession needs an
/// address, so each new service is resolved by opening (and at once closing) a TCP
/// connection to it and reading the remote endpoint. IPv4 only, like the host's pairing
/// links, which keeps link-local IPv6 scope ids out of URLs.
@MainActor
public final class HostBrowser: ObservableObject {
    public static let serviceType = "_perritodeck._tcp"

    @Published public private(set) var hosts: [FoundHost] = []

    private var browser: NWBrowser?
    private var resolving: [String: NWConnection] = [:]
    private var found: [String: FoundHost] = [:]

    public init() {}

    public func start() {
        guard browser == nil else { return }
        let parameters = NWParameters()
        parameters.includePeerToPeer = false
        let browser = NWBrowser(for: .bonjourWithTXTRecord(type: Self.serviceType, domain: nil), using: parameters)
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            Task { @MainActor [weak self] in self?.update(results) }
        }
        browser.stateUpdateHandler = { [weak self] state in
            if case .failed = state {
                Task { @MainActor [weak self] in self?.stop() }
            }
        }
        self.browser = browser
        browser.start(queue: .main)
    }

    public func stop() {
        browser?.cancel()
        browser = nil
        for connection in resolving.values { connection.cancel() }
        resolving.removeAll()
    }

    private func update(_ results: Set<NWBrowser.Result>) {
        guard browser != nil else { return }
        var present = Set<String>()
        for result in results {
            guard case let .service(name, _, _, _) = result.endpoint else { continue }
            present.insert(name)
            var txt: NWTXTRecord?
            if case let .bonjour(record) = result.metadata { txt = record }
            let hostId = txt?["id"]
            let displayName = txt?["n"] ?? name
            if found[name] == nil && resolving[name] == nil {
                resolve(result.endpoint, serviceName: name, name: displayName, hostId: hostId)
            }
        }
        // Services that went away.
        for name in Array(found.keys) where !present.contains(name) {
            found[name] = nil
        }
        for (name, connection) in resolving where !present.contains(name) {
            connection.cancel()
            resolving[name] = nil
        }
        publish()
    }

    private func resolve(_ endpoint: NWEndpoint, serviceName: String, name: String, hostId: String?) {
        let parameters = NWParameters.tcp
        if let ip = parameters.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options {
            ip.version = .v4
        }
        let connection = NWConnection(to: endpoint, using: parameters)
        resolving[serviceName] = connection
        connection.stateUpdateHandler = { [weak self, weak connection] state in
            switch state {
            case .ready:
                let remote = connection?.currentPath?.remoteEndpoint
                connection?.cancel()
                Task { @MainActor [weak self] in
                    self?.resolved(serviceName: serviceName, name: name, hostId: hostId, remote: remote)
                }
            case .failed, .waiting:
                connection?.cancel()
                Task { @MainActor [weak self] in self?.resolving[serviceName] = nil }
            case .cancelled:
                connection?.stateUpdateHandler = nil
            default:
                break
            }
        }
        connection.start(queue: .main)
    }

    private func resolved(serviceName: String, name: String, hostId: String?, remote: NWEndpoint?) {
        guard resolving.removeValue(forKey: serviceName) != nil else { return }
        guard case let .hostPort(host, port)? = remote, let address = Self.addressString(host) else { return }
        let entry = FoundHost(serviceName: serviceName, name: name, address: address, port: Int(port.rawValue), hostId: hostId)
        // One entry per computer, even if it re-registered under a new name.
        found = found.filter { $0.value.name != entry.name && (hostId == nil || $0.value.hostId != hostId) }
        found[serviceName] = entry
        publish()
    }

    private func publish() {
        hosts = found.values.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    nonisolated static func addressString(_ host: NWEndpoint.Host) -> String? {
        switch host {
        case .ipv4(let address):
            return "\(address)"
        case .ipv6(let address):
            // Drop any interface scope ("%en0"): it isn't valid in a URL.
            return "\(address)".split(separator: "%").first.map(String.init)
        case .name(let name, _):
            return name
        @unknown default:
            return nil
        }
    }
}
