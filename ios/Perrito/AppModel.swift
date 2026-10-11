import Combine
import Foundation
import PerritoKit

/// Which top-level screen is showing.
enum Route: Equatable {
    case home
    case pair
    case scan
    case deck
}

/// Where the pairing flow is.
enum PairState: Equatable {
    case form(address: String, error: String?)
    case working(String)
    /// Manual pairing: the person compares this fingerprint with the one the computer shows.
    case confirm(host: String, port: Int, code: String, fingerprint: String)
}

/// The app's state and flows. Mirrors android/.../DeckViewModel.kt.
@MainActor
final class AppModel: ObservableObject {
    let settings: Settings
    let discovery: HostBrowser
    let connection: DeckConnection

    @Published private(set) var route: Route = .home
    @Published private(set) var paired: [PairedHost]
    @Published private(set) var pairState: PairState
    /// Shown on the home screen (e.g. why the last connection ended).
    @Published private(set) var notice: String?

    private var pairTask: Task<Void, Never>?
    private var cancellables = Set<AnyCancellable>()

    init(settings: Settings = Settings()) {
        self.settings = settings
        self.discovery = HostBrowser()
        self.connection = DeckConnection()
        self.paired = settings.pairedHosts
        self.pairState = .form(address: settings.lastManualAddress, error: nil)

        connection.$state
            .receive(on: DispatchQueue.main)
            .sink { [weak self] state in
                guard let self else { return }
                if case .failed(let reason, _) = state, self.route == .deck {
                    self.notice = reason
                    self.route = .home
                }
            }
            .store(in: &cancellables)

        // A dedicated deck device should come straight back to its computer after a restart.
        if let id = settings.lastHostId, let host = paired.first(where: { $0.hostId == id }) {
            connect(to: host)
        }
    }

    // MARK: - Hosts

    func connect(to host: PairedHost) {
        // Prefer the address discovery just saw (DHCP may have moved the computer).
        var target = host
        if let seen = discovery.hosts.first(where: { $0.hostId == host.hostId }),
           seen.address != host.address || seen.port != host.port {
            settings.updateAddress(hostId: host.hostId, address: seen.address, port: seen.port)
            target.address = seen.address
            target.port = seen.port
        }
        paired = settings.pairedHosts
        settings.lastHostId = host.hostId
        notice = nil
        // Android adds 127.0.0.1 as a fallback for USB (adb reverse). iOS has no USB
        // transport, so there are no fallbacks; DeckConnection still supports them.
        connection.connect(
            url: target.url,
            hello: DeviceInfo.hello(deviceId: settings.deviceId, secret: target.deviceSecret),
            fingerprint: target.hostId,
            fallbacks: []
        )
        route = .deck
    }

    func disconnect() {
        connection.disconnect()
        settings.lastHostId = nil
        route = .home
    }

    func forget(_ host: PairedHost) {
        settings.forget(hostId: host.hostId)
        paired = settings.pairedHosts
    }

    // MARK: - Pairing

    func openPairing(prefillAddress: String? = nil) {
        pairTask?.cancel()
        pairState = .form(address: prefillAddress ?? settings.lastManualAddress, error: nil)
        route = .pair
    }

    func openScan() {
        pairTask?.cancel()
        route = .scan
    }

    func closePairing() {
        pairTask?.cancel()
        route = .home
    }

    /// Manual pairing step 1: reach the computer and show its fingerprint for comparison.
    func submitCode(address: String, code: String) {
        let digits = code.filter { $0.isASCII && $0.isNumber }
        guard let hp = HostPort.parseTyped(address) else {
            pairState = .form(address: address, error: "Enter the computer's address, like 192.168.1.20")
            return
        }
        guard digits.count == 6 else {
            pairState = .form(address: address, error: "The code has 6 digits")
            return
        }
        settings.lastManualAddress = address.trimmingCharacters(in: .whitespacesAndNewlines)
        pairTask?.cancel()
        pairState = .working("Reaching \(hp.host)…")
        pairTask = Task { [weak self] in
            do {
                let fingerprint = try await Pairing.probe(host: hp.host, port: hp.port)
                guard !Task.isCancelled else { return }
                self?.pairState = .confirm(host: hp.host, port: hp.port, code: digits, fingerprint: fingerprint)
            } catch {
                guard !Task.isCancelled else { return }
                self?.pairState = .form(address: address, error: error.localizedDescription)
            }
        }
    }

    /// Manual pairing step 2: the fingerprints match, so send the code.
    func confirmPairing() {
        guard case let .confirm(host, port, code, fingerprint) = pairState else { return }
        pairTask?.cancel()
        pairTask = Task { [weak self] in
            await self?.finishPairing(host: host, port: port, fingerprint: fingerprint, code: code)
        }
    }

    /// A scanned QR link carries the fingerprint, so no comparison step is needed.
    func handleLink(_ uri: String) {
        guard let link = PairingLink.parse(uri) else {
            notice = "That pairing link isn't valid. Show a new code on the computer and scan again."
            route = .home
            return
        }
        connection.disconnect()
        route = .pair
        pairTask?.cancel()
        pairState = .working("Pairing with \(link.hostName)…")
        pairTask = Task { [weak self] in
            // The host lists every address it has; use the first one this device can reach.
            for address in link.addresses {
                guard let fingerprint = try? await Pairing.probe(host: address.host, port: address.port) else { continue }
                if Task.isCancelled { return }
                guard fingerprint == link.fingerprint else { continue }
                await self?.finishPairing(host: address.host, port: address.port, fingerprint: link.fingerprint, code: link.code)
                return
            }
            guard !Task.isCancelled, let first = link.addresses.first else { return }
            self?.pairState = .form(
                address: first.display,
                error: "Couldn't reach \(link.hostName). Make sure this \(DeviceInfo.model) is on the same Wi-Fi network."
            )
        }
    }

    private func finishPairing(host: String, port: Int, fingerprint: String, code: String) async {
        let address = HostPort(host: host, port: port).display
        pairState = .working("Pairing…")
        do {
            let pairedHost = try await Pairing.pair(
                host: host,
                port: port,
                fingerprint: fingerprint,
                code: code,
                deviceId: settings.deviceId,
                deviceName: DeviceInfo.name
            )
            guard !Task.isCancelled else { return }
            settings.savePaired(pairedHost)
            paired = settings.pairedHosts
            pairState = .form(address: "", error: nil)
            connect(to: pairedHost)
        } catch is TimeoutError {
            guard !Task.isCancelled else { return }
            pairState = .form(address: address, error: "The computer didn't answer. Try again.")
        } catch {
            guard !Task.isCancelled else { return }
            pairState = .form(address: address, error: error.localizedDescription)
        }
    }
}
