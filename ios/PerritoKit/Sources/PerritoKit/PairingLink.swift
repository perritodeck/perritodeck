import Foundation

/// The link the host shows as a QR code:
/// `perritodeck://pair?v=1&n=<name>&a=<ip:port>[&a=…]&fp=<fingerprint>&c=<code>`.
/// The system Camera app opens Perrito with it, or Perrito's own scanner reads it.
/// Mirrors android/.../data/PairingLink.kt.
public struct PairingLink: Equatable, Sendable {
    public var hostName: String
    public var addresses: [HostPort]
    public var fingerprint: String
    public var code: String

    public init(hostName: String, addresses: [HostPort], fingerprint: String, code: String) {
        self.hostName = hostName
        self.addresses = addresses
        self.fingerprint = fingerprint
        self.code = code
    }

    public static let prefix = "perritodeck://pair?"

    public static func parse(_ uri: String) -> PairingLink? {
        guard uri.hasPrefix(prefix) else { return nil }
        let query = uri.dropFirst(prefix.count)
        let params: [(String, String)] = query.split(separator: "&", omittingEmptySubsequences: false).compactMap { part in
            guard let eq = part.firstIndex(of: "="), eq != part.startIndex else { return nil }
            let key = String(part[part.startIndex..<eq])
            let value = String(part[part.index(after: eq)...])
            return (key, decode(value))
        }
        func one(_ key: String) -> String? { params.first { $0.0 == key }?.1 }

        guard one("v") == "1" else { return nil }
        guard let fp = one("fp"), Fingerprint.isWellFormed(fp) else { return nil }
        guard let code = one("c"), isPairingCode(code) else { return nil }
        let addresses = params.filter { $0.0 == "a" }.compactMap { HostPort.parse($0.1) }
        guard !addresses.isEmpty else { return nil }
        return PairingLink(hostName: one("n") ?? "your computer", addresses: addresses, fingerprint: fp, code: code)
    }

    /// Exactly six ASCII digits.
    public static func isPairingCode(_ s: String) -> Bool {
        s.count == 6 && s.allSatisfy { $0.isASCII && $0.isNumber }
    }

    /// application/x-www-form-urlencoded decoding, like Java's URLDecoder: `+` is a space.
    /// A malformed escape leaves the value as it was.
    private static func decode(_ s: String) -> String {
        let spaced = s.replacingOccurrences(of: "+", with: " ")
        return spaced.removingPercentEncoding ?? s
    }
}

public struct HostPort: Equatable, Hashable, Sendable {
    public var host: String
    public var port: Int

    public init(host: String, port: Int) {
        self.host = host
        self.port = port
    }

    /// `1.2.3.4:46710` or `[fe80::1]:46710`.
    public static func parse(_ s: String) -> HostPort? {
        let host: String
        let portText: String
        if s.hasPrefix("[") {
            guard let end = s.firstIndex(of: "]"), end != s.startIndex else { return nil }
            host = String(s[s.index(after: s.startIndex)..<end])
            var rest = s[s.index(after: end)...]
            if rest.hasPrefix(":") { rest = rest.dropFirst() }
            portText = String(rest)
        } else {
            guard let colon = s.lastIndex(of: ":"), colon != s.startIndex else { return nil }
            host = String(s[s.startIndex..<colon])
            portText = String(s[s.index(after: colon)...])
        }
        guard let port = Int(portText), (1...65535).contains(port) else { return nil }
        guard !host.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return HostPort(host: host, port: port)
    }

    /// What a person types on the pairing screen: an address with an optional port.
    public static func parseTyped(_ s: String) -> HostPort? {
        let t = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return nil }
        return parse(t) ?? parse("\(t):\(defaultPort)")
    }

    /// `host:port`, with IPv6 literals bracketed.
    public var display: String { "\(formatHost(host)):\(port)" }
}

/// Bracket bare IPv6 literals so they form a valid URL.
public func formatHost(_ host: String) -> String {
    host.contains(":") && !host.hasPrefix("[") ? "[\(host)]" : host
}

/// The WebSocket URL for a host.
public func webSocketURLString(host: String, port: Int) -> String {
    "wss://\(formatHost(host)):\(port)"
}
