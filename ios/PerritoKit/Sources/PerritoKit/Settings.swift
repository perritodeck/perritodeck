import Foundation
import Security

/// A computer this device has paired with.
public struct PairedHost: Codable, Equatable, Identifiable, Sendable {
    /// Certificate fingerprint; the pin for every connection.
    public var hostId: String
    public var name: String
    /// Last address that worked (discovery may supply a newer one).
    public var address: String
    public var port: Int
    public var deviceSecret: String

    public var id: String { hostId }
    public var url: String { webSocketURLString(host: address, port: port) }

    public init(hostId: String, name: String, address: String, port: Int, deviceSecret: String) {
        self.hostId = hostId
        self.name = name
        self.address = address
        self.port = port
        self.deviceSecret = deviceSecret
    }
}

/// Small secrets by name. The app uses the Keychain; tests use memory.
public protocol SecureStore {
    func data(for account: String) -> Data?
    func set(_ data: Data?, for account: String)
}

/// Generic-password items in the Keychain, readable only on this device after its first
/// unlock (not synced to iCloud, not restored onto another device).
public struct KeychainStore: SecureStore {
    public let service: String

    public init(service: String) {
        self.service = service
    }

    private func baseQuery(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }

    public func data(for account: String) -> Data? {
        var query = baseQuery(account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }

    public func set(_ data: Data?, for account: String) {
        let query = baseQuery(account)
        guard let data else {
            SecItemDelete(query as CFDictionary)
            return
        }
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            let add = query.merging(attributes) { _, new in new }
            SecItemAdd(add as CFDictionary, nil)
        }
    }
}

/// In-memory store for tests and previews.
public final class MemorySecureStore: SecureStore {
    private var items: [String: Data] = [:]

    public init() {}

    public func data(for account: String) -> Data? { items[account] }

    public func set(_ data: Data?, for account: String) { items[account] = data }
}

/// The device id and paired hosts (secrets, so in the Keychain) plus a few preferences
/// (UserDefaults). Mirrors android/.../data/Settings.kt.
public final class Settings {
    private let secure: SecureStore
    private let defaults: UserDefaults

    private enum Key {
        static let deviceId = "deviceId"
        static let pairedHosts = "pairedHosts"
        static let lastHostId = "lastHostId"
        static let lastManualAddress = "lastManualAddress"
        static let installed = "installed"
    }

    public init(
        secure: SecureStore = KeychainStore(service: "dev.perritodeck.client"),
        defaults: UserDefaults = .standard
    ) {
        self.secure = secure
        self.defaults = defaults
        // Keychain items outlive the app. After a reinstall start fresh, as Android does
        // (UserDefaults are removed with the app, so the marker is missing).
        if !defaults.bool(forKey: Key.installed) {
            secure.set(nil, for: Key.deviceId)
            secure.set(nil, for: Key.pairedHosts)
            defaults.set(true, forKey: Key.installed)
        }
    }

    /// Stable id the host uses to recognise this device across reconnects.
    public var deviceId: String {
        if let data = secure.data(for: Key.deviceId), let id = String(data: data, encoding: .utf8), !id.isEmpty {
            return id
        }
        let id = UUID().uuidString.lowercased()
        secure.set(Data(id.utf8), for: Key.deviceId)
        return id
    }

    public private(set) var pairedHosts: [PairedHost] {
        get {
            guard let data = secure.data(for: Key.pairedHosts) else { return [] }
            return (try? JSONDecoder().decode([PairedHost].self, from: data)) ?? []
        }
        set {
            secure.set(try? JSONEncoder().encode(newValue), for: Key.pairedHosts)
        }
    }

    /// The host to reconnect to automatically when the app starts.
    public var lastHostId: String? {
        get { defaults.string(forKey: Key.lastHostId) }
        set { defaults.set(newValue, forKey: Key.lastHostId) }
    }

    /// Address and port typed last time on the pairing screen.
    public var lastManualAddress: String {
        get { defaults.string(forKey: Key.lastManualAddress) ?? "" }
        set { defaults.set(newValue, forKey: Key.lastManualAddress) }
    }

    public func savePaired(_ host: PairedHost) {
        pairedHosts = pairedHosts.filter { $0.hostId != host.hostId } + [host]
        lastHostId = host.hostId
    }

    public func updateAddress(hostId: String, address: String, port: Int) {
        pairedHosts = pairedHosts.map { h in
            guard h.hostId == hostId else { return h }
            var updated = h
            updated.address = address
            updated.port = port
            return updated
        }
    }

    public func forget(hostId: String) {
        pairedHosts = pairedHosts.filter { $0.hostId != hostId }
        if lastHostId == hostId { lastHostId = nil }
    }
}
