import CryptoKit
import Foundation

/// The host uses a self-signed certificate; instead of a CA chain we pin the SHA-256 of
/// its certificate (DER), base64url without padding, learned when pairing. Same format as
/// the host's `HostIdentity::fingerprint` and Android's `Fingerprint`.
public enum Fingerprint {
    public static func of(certificateDER: Data) -> String {
        Base64URL.encode(Data(SHA256.hash(data: certificateDER)))
    }

    /// Short form for comparing by eye, e.g. `3FA9 07C2 D41E` (first 6 bytes). Matches the host.
    public static func display(_ fingerprint: String) -> String {
        guard let bytes = Base64URL.decode(fingerprint) else { return "?" }
        let hex = bytes.prefix(6).map { String(format: "%02X", $0) }.joined()
        var groups: [String] = []
        var rest = Substring(hex)
        while !rest.isEmpty {
            groups.append(String(rest.prefix(4)))
            rest = rest.dropFirst(4)
        }
        return groups.joined(separator: " ")
    }

    /// A pairing-link or `paired.hostId` fingerprint: 43 base64url characters (32 bytes).
    public static func isWellFormed(_ s: String) -> Bool {
        s.count == 43 && s.allSatisfy { Base64URL.isAlphabet($0) }
    }
}

/// base64url without padding (RFC 4648 §5).
public enum Base64URL {
    public static func encode(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    /// Nil if `s` has characters outside the base64url alphabet or an impossible length.
    public static func decode(_ s: String) -> Data? {
        guard s.allSatisfy(isAlphabet) else { return nil }
        var standard = s
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let remainder = standard.count % 4
        if remainder == 1 { return nil }
        if remainder > 0 { standard += String(repeating: "=", count: 4 - remainder) }
        return Data(base64Encoded: standard)
    }

    static func isAlphabet(_ c: Character) -> Bool {
        guard c.isASCII else { return false }
        return c.isLetter || c.isNumber || c == "-" || c == "_"
    }
}
