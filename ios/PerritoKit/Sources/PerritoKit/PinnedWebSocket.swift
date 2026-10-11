import Foundation
import Security

/// What happened on a WebSocket, in order.
public enum SocketEvent: Sendable, Equatable {
    case open
    case text(String)
    /// The connection closed; the stream ends after this.
    case closed(reason: String)
    /// The connection failed; the stream ends after this. `pinMismatch` is true when the
    /// host presented a different certificate from the one we paired with.
    case failure(reason: String, pinMismatch: Bool)
}

/// How a `PinnedWebSocket` treats the host's self-signed certificate.
public enum PinPolicy: Sendable, Equatable {
    /// Accept exactly the certificate with this fingerprint (every normal connection).
    case expect(String)
    /// Record the fingerprint and stop at the TLS handshake (`Pairing.probe`).
    case probe
}

/// One `wss://` connection to a host, with the host certificate pinned by fingerprint like
/// Android's `PinningTrustManager`: the SHA-256 of the leaf certificate's DER, base64url.
///
/// Events arrive on `events` (an AsyncStream that ends after `.closed` or `.failure`).
/// `send` is fire-and-forget and keeps order; a failed send surfaces as a failure event.
/// Call `close()` when done — the URLSession holds its delegate (this object) until then.
public final class PinnedWebSocket: NSObject, URLSessionWebSocketDelegate, @unchecked Sendable {
    public static let pinMismatchMessage =
        "This isn't the computer you paired with — its identity changed. If you reinstalled PerritoDeck Host, pair again."

    public let events: AsyncStream<SocketEvent>
    private let continuation: AsyncStream<SocketEvent>.Continuation
    private let pin: PinPolicy
    private let url: URL
    private let timeout: TimeInterval

    private let lock = NSLock()
    private var session: URLSession?
    private var task: URLSessionWebSocketTask?
    private var finished = false
    private var _seenFingerprint: String?

    /// The fingerprint the host presented, once the TLS handshake got that far.
    public var seenFingerprint: String? {
        lock.lock()
        defer { lock.unlock() }
        return _seenFingerprint
    }

    /// - Parameter timeout: how long the connection may take to open (and the longest gap
    ///   between packets URLSession tolerates; the protocol's heartbeats keep traffic flowing).
    public init(url: URL, pin: PinPolicy, timeout: TimeInterval = 10) {
        var c: AsyncStream<SocketEvent>.Continuation?
        events = AsyncStream(bufferingPolicy: .unbounded) { c = $0 }
        continuation = c!
        self.pin = pin
        self.url = url
        self.timeout = timeout
        super.init()
    }

    /// Open the connection. Call once.
    public func start() {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = timeout
        config.waitsForConnectivity = false
        config.urlCache = nil
        config.httpCookieStorage = nil
        // A dedicated session per connection, so a pinning decision never leaks between hosts.
        let session = URLSession(configuration: config, delegate: self, delegateQueue: nil)
        // No Origin header: the host refuses WebSocket upgrades that carry one.
        let request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: timeout)
        let task = session.webSocketTask(with: request)
        lock.lock()
        self.session = session
        self.task = task
        lock.unlock()
        task.resume()
        receiveNext(task)
    }

    /// Send one text frame. Errors end the connection with a `.failure` event.
    public func send(_ text: String) {
        lock.lock()
        let task = self.task
        lock.unlock()
        task?.send(.string(text)) { [weak self] error in
            if let error {
                self?.finish(.failure(reason: Self.describe(error), pinMismatch: false))
            }
        }
    }

    /// End the connection from our side with a `.failure(reason)` event (e.g. the host
    /// stopped answering heartbeats), then close.
    public func abort(reason: String) {
        finish(.failure(reason: reason, pinMismatch: false))
        close()
    }

    /// Close the connection and release the session. Safe to call more than once.
    public func close() {
        lock.lock()
        let task = self.task
        let session = self.session
        self.task = nil
        self.session = nil
        lock.unlock()
        task?.cancel(with: .normalClosure, reason: nil)
        session?.invalidateAndCancel()
        finish(nil)
    }

    // MARK: - Receiving

    private func receiveNext(_ task: URLSessionWebSocketTask) {
        task.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let message):
                switch message {
                case .string(let text):
                    self.yield(.text(text))
                case .data:
                    break // the protocol uses text frames only
                @unknown default:
                    break
                }
                self.receiveNext(task)
            case .failure(let error):
                self.finish(.failure(reason: Self.describe(error), pinMismatch: false))
            }
        }
    }

    private func yield(_ event: SocketEvent) {
        lock.lock()
        let done = finished
        lock.unlock()
        if !done { continuation.yield(event) }
    }

    /// Deliver a final event (if any) and end the stream; later events are dropped.
    private func finish(_ event: SocketEvent?) {
        lock.lock()
        let already = finished
        finished = true
        lock.unlock()
        guard !already else { return }
        if let event { continuation.yield(event) }
        continuation.finish()
    }

    private static func describe(_ error: Error) -> String {
        let ns = error as NSError
        if ns.domain == NSURLErrorDomain && ns.code == NSURLErrorCancelled { return "connection cancelled" }
        return error.localizedDescription
    }

    // MARK: - URLSessionWebSocketDelegate

    public func urlSession(
        _ session: URLSession,
        webSocketTask: URLSessionWebSocketTask,
        didOpenWithProtocol negotiatedProtocol: String?
    ) {
        yield(.open)
    }

    public func urlSession(
        _ session: URLSession,
        webSocketTask: URLSessionWebSocketTask,
        didCloseWith closeCode: URLSessionWebSocketTask.CloseCode,
        reason: Data?
    ) {
        let text = reason.flatMap { String(data: $0, encoding: .utf8) } ?? ""
        finish(.closed(reason: text.isEmpty ? "host closed the connection" : text))
    }

    public func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error {
            finish(.failure(reason: Self.describe(error), pinMismatch: false))
        } else {
            finish(.closed(reason: "connection closed"))
        }
    }

    // MARK: - Certificate pinning

    public func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust
        else {
            // Client certificates and HTTP auth are not used.
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        guard let fingerprint = Self.leafFingerprint(trust) else {
            finish(.failure(reason: "host sent no certificate", pinMismatch: false))
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        lock.lock()
        _seenFingerprint = fingerprint
        lock.unlock()

        switch pin {
        case .expect(let expected) where expected == fingerprint:
            // The certificate is pinned, so its name and issuer don't matter (hosts are reached by IP).
            completionHandler(.useCredential, URLCredential(trust: trust))
        case .expect:
            finish(.failure(reason: Self.pinMismatchMessage, pinMismatch: true))
            completionHandler(.cancelAuthenticationChallenge, nil)
        case .probe:
            // We have what we came for; don't open a WebSocket.
            finish(.failure(reason: "probe finished", pinMismatch: false))
            completionHandler(.cancelAuthenticationChallenge, nil)
        }
    }

    /// SHA-256 fingerprint of the leaf (first) certificate the host presented.
    static func leafFingerprint(_ trust: SecTrust) -> String? {
        guard let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = chain.first
        else { return nil }
        let der = SecCertificateCopyData(leaf) as Data
        return Fingerprint.of(certificateDER: der)
    }
}
