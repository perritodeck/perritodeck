import Combine
import CoreGraphics
import Dispatch
import Foundation

/// A short-lived ✓ / ⚠ on a key after its action finished. `serial` makes two flashes on
/// the same key distinct.
public struct KeyFlash: Equatable, Sendable {
    public var keyIndex: Int
    public var ok: Bool
    public var serial: Int

    public init(keyIndex: Int, ok: Bool, serial: Int) {
        self.keyIndex = keyIndex
        self.ok = ok
        self.serial = serial
    }
}

/// One logical connection to a host: connects, performs the hello handshake, keeps the
/// layout up to date, sends key events, heartbeats, and reconnects with backoff.
/// Mirrors android/.../net/DeckConnection.kt. Lives on the main actor; SwiftUI observes it.
@MainActor
public final class DeckConnection: ObservableObject {
    public enum State: Equatable, Sendable {
        case idle
        case connecting(url: String, attempt: Int, lastError: String?)
        case connected(Welcome)
        /// The host refused us for a reason retrying won't fix (not paired, wrong version, wrong host).
        case failed(reason: String, notPaired: Bool)

        public var isConnected: Bool {
            if case .connected = self { return true }
            return false
        }
    }

    @Published public private(set) var state: State = .idle
    /// Last layout received; kept while reconnecting so the grid stays on screen.
    @Published public private(set) var layout: Layout?
    /// Host-rendered key images by key index (from `setImage`); cleared with each new layout.
    @Published public private(set) var images: [Int: CGImage] = [:]
    @Published public private(set) var flash: KeyFlash?
    /// Round trip of the most recent key press, in milliseconds.
    @Published public private(set) var rttMs: Double?

    private let decodeImage: (String) -> CGImage?
    private var loop: Task<Void, Never>?
    private var heartbeat: Task<Void, Never>?
    private var socket: PinnedWebSocket?
    private var nextId = 1
    private var pendingAcks: [Int: UInt64] = [:]
    private var lastHeardNs: UInt64 = 0
    private var flashSerial = 0
    private var throttle = SliderThrottle()

    public init(decodeImage: @escaping (String) -> CGImage? = KeyImages.decode) {
        self.decodeImage = decodeImage
    }

    // MARK: - Public API

    /// Connect and stay connected to the host whose certificate has `fingerprint`.
    /// `fallbacks` are other addresses for the same host; retries alternate between them,
    /// and the one that worked is tried first next time. (Android uses this for USB via
    /// 127.0.0.1; iOS has no USB transport, so the app passes none today.)
    public func connect(url: String, hello: Hello, fingerprint: String, fallbacks: [String] = []) {
        disconnect()
        layout = nil
        images = [:]
        let schedule = ReconnectSchedule(urls: [url] + fallbacks)
        loop = Task { [weak self] in
            await self?.runLoop(schedule: schedule, hello: hello, fingerprint: fingerprint)
        }
    }

    public func disconnect() {
        loop?.cancel()
        loop = nil
        heartbeat?.cancel()
        heartbeat = nil
        socket?.close()
        socket = nil
        pendingAcks.removeAll()
        throttle.reset()
        state = .idle
    }

    public func keyDown(_ index: Int) {
        sendKey(.keyDown(KeyEvent(keyIndex: index, clientTimestampMs: Self.wallClockMs())))
    }

    public func keyUp(_ index: Int) {
        sendKey(.keyUp(KeyEvent(keyIndex: index, clientTimestampMs: Self.wallClockMs())))
    }

    /// A slider moved to `value` (0–100). Sent at most every 50 ms while
    /// dragging (the latest value wins), and always when `final` (the finger lifted).
    public func sliderChange(_ index: Int, value: Int, final: Bool) {
        let v = clampPercent(value)
        setSliderValue(index, v)
        switch throttle.change(index: index, value: v, final: final, nowMs: Self.monotonicMs()) {
        case .send(let value):
            sendSlider(index, value)
        case .schedule(let afterMs):
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: UInt64(afterMs) * 1_000_000)
                guard let self, let pending = self.throttle.takePending(index: index) else { return }
                self.sendSlider(index, pending)
            }
        case .coalesced:
            break
        }
    }

    // MARK: - Sending

    private func sendSlider(_ index: Int, _ value: Int) {
        guard let ws = socket, state.isConnected else { return }
        throttle.markSent(index: index, nowMs: Self.monotonicMs())
        ws.send(Codec.encode(Envelope(.sliderChange(SliderValue(keyIndex: index, value: value)))))
    }

    private func sendKey(_ message: Message) {
        guard let ws = socket, state.isConnected else { return }
        let id = takeId()
        if pendingAcks.count > 256 { pendingAcks.removeAll() }
        pendingAcks[id] = Self.monotonicNs()
        ws.send(Codec.encode(Envelope(message, id: id)))
    }

    private func takeId() -> Int {
        let id = nextId
        nextId = nextId == Int(UInt32.max) ? 1 : nextId + 1
        return id
    }

    // MARK: - Session loop

    private enum SessionEnd {
        case retry(reason: String, wasConnected: Bool)
        case fatal(reason: String, notPaired: Bool)
    }

    private func runLoop(schedule initial: ReconnectSchedule, hello: Hello, fingerprint: String) async {
        var schedule = initial
        var lastError: String?
        while !Task.isCancelled {
            let url = schedule.next()
            state = .connecting(url: url, attempt: schedule.attempt, lastError: lastError)
            let end = await runSession(url: url, hello: hello, fingerprint: fingerprint)
            if Task.isCancelled { return }
            switch end {
            case .fatal(let reason, let notPaired):
                state = .failed(reason: reason, notPaired: notPaired)
                return
            case .retry(let reason, let wasConnected):
                lastError = reason
                let waitMs = schedule.ended(wasConnected: wasConnected)
                if waitMs > 0 {
                    try? await Task.sleep(nanoseconds: UInt64(waitMs) * 1_000_000)
                }
            }
        }
    }

    private func runSession(url: String, hello: Hello, fingerprint: String) async -> SessionEnd {
        guard let u = URL(string: url), u.scheme == "wss" else {
            return .fatal(reason: "Invalid address: \(url)", notPaired: false)
        }
        let ws = PinnedWebSocket(url: u, pin: .expect(fingerprint))
        socket = ws
        var connected = false
        lastHeardNs = Self.monotonicNs()
        defer {
            ws.close()
            // After disconnect() or a newer connect(), this session no longer owns the shared state.
            if socket === ws {
                heartbeat?.cancel()
                heartbeat = nil
                socket = nil
                pendingAcks.removeAll()
            }
        }
        ws.start()

        for await event in ws.events {
            if Task.isCancelled { break }
            switch event {
            case .open:
                ws.send(Codec.encode(Envelope(.hello(hello), id: takeId())))
            case .failure(let reason, let pinMismatch):
                if pinMismatch { return .fatal(reason: reason, notPaired: false) }
                return .retry(reason: reason, wasConnected: connected)
            case .closed(let reason):
                return .retry(reason: reason, wasConnected: connected)
            case .text(let text):
                lastHeardNs = Self.monotonicNs()
                let decoded: Decoded
                do {
                    decoded = try Codec.decode(text)
                } catch ProtocolError.unsupportedVersion(_) {
                    return .fatal(reason: "PerritoDeck on the computer speaks a newer protocol. Update Perrito.", notPaired: false)
                } catch {
                    continue // a single bad frame isn't worth dropping the session
                }
                guard case .known(let envelope) = decoded else { continue }
                if let end = handle(envelope, ws: ws, connected: &connected) { return end }
            }
        }
        return .retry(reason: "connection closed", wasConnected: connected)
    }

    /// Apply one message from the host. Returns a session end for errors that stop the session.
    private func handle(_ envelope: Envelope, ws: PinnedWebSocket, connected: inout Bool) -> SessionEnd? {
        switch envelope.message {
        case .welcome(let welcome):
            connected = true
            state = .connected(welcome)
            startHeartbeat(ws, intervalMs: welcome.heartbeatIntervalMs)
        case .layout(let newLayout):
            layout = newLayout
            images = [:]
        case .showOk(let feedback):
            showFlash(feedback.keyIndex, ok: true)
        case .showAlert(let feedback):
            showFlash(feedback.keyIndex, ok: false)
        case .setValue(let slider):
            setSliderValue(slider.keyIndex, clampPercent(slider.value))
        case .setImage(let payload):
            if let dataURL = payload.image, let image = decodeImage(dataURL) {
                images[payload.keyIndex] = image
            } else {
                images[payload.keyIndex] = nil
            }
        case .setTitle(let payload):
            updateKey(payload.keyIndex) { $0.title = payload.title }
        case .keyAck:
            if let id = envelope.id, let sent = pendingAcks.removeValue(forKey: id) {
                rttMs = Double(Self.monotonicNs() &- sent) / 1_000_000
            }
        case .error(let error):
            switch error.code {
            case .unauthorized, .unsupportedVersion:
                return .fatal(reason: error.message, notPaired: false)
            case .notPaired:
                return .fatal(reason: "This computer no longer recognises this device. Pair it again.", notPaired: true)
            default:
                break // non-fatal
            }
        default:
            break // heartbeatAck only refreshes lastHeardNs
        }
        return nil
    }

    private func startHeartbeat(_ ws: PinnedWebSocket, intervalMs: Int) {
        heartbeat?.cancel()
        let interval = UInt64(min(max(intervalMs, 250), 60_000))
        heartbeat = Task { [weak self] in
            var seq = 0
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: interval * 1_000_000)
                guard let self, !Task.isCancelled else { return }
                // No traffic for 3 intervals: the host is gone (Wi-Fi dropped, computer slept). Force a reconnect.
                if (Self.monotonicNs() &- self.lastHeardNs) / 1_000_000 > interval * 3 {
                    ws.abort(reason: "The computer stopped answering.")
                    return
                }
                seq += 1
                ws.send(Codec.encode(Envelope(.heartbeat(Heartbeat(seq: seq, sentAtMs: Self.wallClockMs())))))
            }
        }
    }

    // MARK: - State helpers

    private func showFlash(_ index: Int, ok: Bool) {
        flashSerial += 1
        flash = KeyFlash(keyIndex: index, ok: ok, serial: flashSerial)
    }

    private func setSliderValue(_ index: Int, _ value: Int) {
        guard let current = layout,
              let i = current.keys.firstIndex(where: { $0.index == index && $0.isSlider })
        else { return }
        layout?.keys[i].value = value
    }

    private func updateKey(_ index: Int, _ change: (inout KeySpec) -> Void) {
        guard let i = layout?.keys.firstIndex(where: { $0.index == index }) else { return }
        change(&layout!.keys[i])
    }

    // MARK: - Clocks

    /// Wall clock, Unix ms (informational only in the protocol).
    nonisolated static func wallClockMs() -> Int64 {
        Int64((Date().timeIntervalSince1970 * 1000).rounded())
    }

    /// Monotonic clock for measuring round trips and silence.
    nonisolated static func monotonicNs() -> UInt64 {
        DispatchTime.now().uptimeNanoseconds
    }

    nonisolated static func monotonicMs() -> Int64 {
        Int64(monotonicNs() / 1_000_000)
    }
}
