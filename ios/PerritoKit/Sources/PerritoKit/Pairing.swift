import Foundation

/// A pairing step failed; `message` is written for the person holding the device.
public struct PairingError: Error, LocalizedError, Equatable, Sendable {
    public let message: String

    public init(_ message: String) {
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// An operation took longer than its time limit.
public struct TimeoutError: Error, Equatable, Sendable {
    public init() {}
}

/// Run `operation`, cancelling it and throwing `TimeoutError` after `seconds`.
public func withTimeout<T: Sendable>(
    seconds: Double,
    _ operation: @escaping @Sendable () async throws -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await operation() }
        group.addTask {
            try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
            throw TimeoutError()
        }
        defer { group.cancelAll() }
        guard let result = try await group.next() else { throw TimeoutError() }
        return result
    }
}

/// Pairing with a host, mirroring android/.../net/Pairing.kt:
/// 1. `probe` reaches the host and learns its certificate fingerprint;
/// 2. the person compares it with the computer's screen (a QR link carries it instead);
/// 3. `pair` sends the one-time code over a connection pinned to that fingerprint.
public enum Pairing {
    /// Reach the host and return its certificate fingerprint (for the person to compare).
    public static func probe(host: String, port: Int, timeout: Double = 5) async throws -> String {
        let reach = "Couldn't reach \(host):\(port) — is PerritoDeck Host running, and on the same network?"
        guard let url = URL(string: webSocketURLString(host: host, port: port)) else {
            throw PairingError("\(reach) (invalid address)")
        }
        do {
            return try await withTimeout(seconds: timeout) {
                let ws = PinnedWebSocket(url: url, pin: .probe, timeout: timeout)
                defer { ws.close() }
                ws.start()
                var lastReason = "no certificate received"
                for await event in ws.events {
                    switch event {
                    case .failure(let reason, _): lastReason = reason
                    case .closed(let reason): lastReason = reason
                    case .open, .text: break
                    }
                }
                if let fingerprint = ws.seenFingerprint { return fingerprint }
                throw PairingError(lastReason)
            }
        } catch let error as PairingError {
            throw PairingError("\(reach) (\(error.message))")
        } catch is TimeoutError {
            throw PairingError("\(reach) (timed out)")
        }
    }

    /// Send the one-time code over a connection pinned to `fingerprint`. On success the
    /// host closes the connection and the client reconnects normally with the issued secret.
    /// - Throws: `PairingError`, or `TimeoutError` if the host doesn't answer within 10 s.
    public static func pair(
        host: String,
        port: Int,
        fingerprint: String,
        code: String,
        deviceId: String,
        deviceName: String
    ) async throws -> PairedHost {
        guard let url = URL(string: webSocketURLString(host: host, port: port)) else {
            throw PairingError("Invalid address: \(host):\(port)")
        }
        let request = PairRequest(code: code, deviceId: deviceId, deviceName: deviceName)
        let reply: Message = try await withTimeout(seconds: 10) {
            let ws = PinnedWebSocket(url: url, pin: .expect(fingerprint))
            defer { ws.close() }
            ws.start()
            for await event in ws.events {
                switch event {
                case .open:
                    ws.send(Codec.encode(Envelope(.pair(request), id: 1)))
                case .text(let text):
                    if case .known(let envelope)? = try? Codec.decode(text) {
                        return envelope.message
                    }
                case .closed:
                    throw PairingError("The computer closed the connection.")
                case .failure(let reason, _):
                    throw PairingError(reason)
                }
            }
            try Task.checkCancellation()
            throw PairingError("The computer closed the connection.")
        }
        switch reply {
        case .paired(let p):
            guard p.hostId == fingerprint else {
                throw PairingError("The computer's identity changed during pairing. Try again.")
            }
            return PairedHost(hostId: fingerprint, name: p.hostName, address: host, port: port, deviceSecret: p.deviceSecret)
        case .error(let e):
            throw PairingError(capitalizingFirst(e.message))
        default:
            throw PairingError("Unexpected reply from the computer (\(reply.type)).")
        }
    }

    static func capitalizingFirst(_ s: String) -> String {
        guard let first = s.first else { return s }
        return first.uppercased() + s.dropFirst()
    }
}
