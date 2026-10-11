import Foundation

/// How the deck grid fits the screen, matching the Android app: 8 pt padding around the
/// grid, 8 pt gaps between keys, square keys as large as fit, never smaller than 16 pt.
public struct GridMetrics: Equatable, Sendable {
    public var cols: Int
    public var rows: Int
    /// Side of one square key.
    public var cell: Double
    public var gap: Double

    public static let defaultPadding = 8.0
    public static let defaultGap = 8.0
    public static let defaultMinimumCell = 16.0

    /// Width and height of the whole grid (without padding).
    public var width: Double { cell * Double(cols) + gap * Double(cols - 1) }
    public var height: Double { cell * Double(rows) + gap * Double(rows - 1) }

    /// - Parameters:
    ///   - width, height: the space available, before padding.
    public static func fit(
        width: Double,
        height: Double,
        cols: Int,
        rows: Int,
        padding: Double = GridMetrics.defaultPadding,
        gap: Double = GridMetrics.defaultGap,
        minimumCell: Double = GridMetrics.defaultMinimumCell
    ) -> GridMetrics {
        let c = max(cols, 1)
        let r = max(rows, 1)
        let innerW = width - padding * 2
        let innerH = height - padding * 2
        let byWidth = (innerW - gap * Double(c - 1)) / Double(c)
        let byHeight = (innerH - gap * Double(r - 1)) / Double(r)
        let cell = max(min(byWidth, byHeight), minimumCell)
        return GridMetrics(cols: c, rows: r, cell: cell, gap: gap)
    }

    /// Keys are numbered row-major: `row * cols + col`.
    public func keyIndex(row: Int, col: Int) -> Int { row * cols + col }
}

public enum SliderMath {
    /// Slider position for a touch at `y` (0 = top) in a slider `height` tall: 100 at the
    /// top, 0 at the bottom, clamped. Truncates like the Android app.
    public static func value(atY y: Double, height: Double) -> Int {
        guard height > 0, y.isFinite else { return 0 }
        let raw = (1 - y / height) * 100
        guard raw.isFinite else { return 0 }
        // Clamp before converting so a wild touch can't overflow Int.
        return Int(min(max(raw, 0), 100).rounded(.towardZero))
    }

    /// Haptic "detents": true when moving from `old` to `new` crosses a multiple of ten.
    public static func crossesTick(from old: Int, to new: Int) -> Bool {
        old / 10 != new / 10
    }
}

public func clampPercent(_ v: Int) -> Int { min(max(v, 0), 100) }

/// Throttles slider moves like the Android app: at most one `sliderChange` per
/// `intervalMs` per slider while dragging (the latest value wins), and always one when
/// the finger lifts. Pure state; the caller owns the clock and the timer.
public struct SliderThrottle: Sendable {
    public enum Decision: Equatable, Sendable {
        /// Send this value now (then call `markSent`).
        case send(Int)
        /// Too soon: the value is held. Call `takePending` after this many milliseconds.
        case schedule(afterMs: Int)
        /// Too soon, and a flush is already scheduled; the held value was replaced.
        case coalesced
    }

    /// Android sends at most ~20 a second while dragging.
    public static let defaultIntervalMs = 50

    public let intervalMs: Int
    private var pending: [Int: Int] = [:]
    private var lastSentMs: [Int: Int64] = [:]

    public init(intervalMs: Int = SliderThrottle.defaultIntervalMs) {
        self.intervalMs = intervalMs
    }

    /// A slider moved. `nowMs` is a monotonic clock in milliseconds.
    public mutating func change(index: Int, value: Int, final: Bool, nowMs: Int64) -> Decision {
        let v = clampPercent(value)
        let tooSoon = lastSentMs[index].map { nowMs - $0 < Int64(intervalMs) } ?? false
        if final || !tooSoon {
            pending[index] = nil
            return .send(v)
        }
        let alreadyScheduled = pending[index] != nil
        pending[index] = v
        return alreadyScheduled ? .coalesced : .schedule(afterMs: intervalMs)
    }

    /// The held value for a slider, if any, removing it.
    public mutating func takePending(index: Int) -> Int? {
        pending.removeValue(forKey: index)
    }

    /// Record that a value actually went out.
    public mutating func markSent(index: Int, nowMs: Int64) {
        lastSentMs[index] = nowMs
    }

    public mutating func reset() {
        pending.removeAll()
        lastSentMs.removeAll()
    }
}

/// Reconnect delays: 0.5 s, 1 s, 2 s, 4 s, then 8 s between attempts (same as Android).
public enum Backoff {
    public static func ms(attempt: Int) -> Int {
        min(250 << min(max(attempt, 1), 5), 8_000)
    }
}

/// Which address to try next, and how long to wait first. Mirrors DeckConnection.runLoop
/// on Android: the addresses for one host (the main one plus fallbacks) are tried in
/// turn without waiting, then the whole round backs off; the address that last worked
/// is tried first afterwards.
public struct ReconnectSchedule: Sendable {
    public let urls: [String]
    public private(set) var attempt = 0
    private var first = 0
    private var current = 0

    /// `urls` must not be empty; duplicates are dropped, keeping the first.
    public init(urls: [String]) {
        var seen = Set<String>()
        let unique = urls.filter { seen.insert($0).inserted }
        precondition(!unique.isEmpty, "ReconnectSchedule needs at least one address")
        self.urls = unique
    }

    /// Start the next attempt; returns the address to try.
    public mutating func next() -> String {
        attempt += 1
        current = (first + attempt - 1) % urls.count
        return urls[current]
    }

    /// The attempt ended (to be retried). Returns how long to wait before `next()`, in ms
    /// (0 means try the next address straight away).
    public mutating func ended(wasConnected: Bool) -> Int {
        if wasConnected {
            attempt = 0
            first = current
        }
        if urls.count > 1 && attempt % urls.count != 0 { return 0 }
        return Backoff.ms(attempt: attempt / urls.count)
    }
}
