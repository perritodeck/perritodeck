import Foundation
import XCTest
@testable import PerritoKit

final class DeckMathTests: XCTestCase {
    // MARK: Grid layout

    func testGridFitsTheNarrowerSide() {
        // Landscape iPad, 5×3: width is the limit. Inner 1008×752 after 8 pt padding.
        let m = GridMetrics.fit(width: 1024, height: 768, cols: 5, rows: 3)
        XCTAssertEqual(m.cell, (1008 - 4 * 8) / 5, accuracy: 0.0001)
        XCTAssertEqual(m.width, 1008, accuracy: 0.0001)
        XCTAssertLessThanOrEqual(m.height, 752)

        // Portrait phone, 3×5: height is the limit. Inner 374×796.
        let p = GridMetrics.fit(width: 390, height: 812, cols: 3, rows: 5)
        XCTAssertEqual(p.cell, min((374 - 16) / 3, (796 - 32) / 5), accuracy: 0.0001)
        XCTAssertLessThanOrEqual(p.width, 374)
        XCTAssertLessThanOrEqual(p.height, 796 + 0.0001)
    }

    func testGridKeysAreNeverTiny() {
        let m = GridMetrics.fit(width: 40, height: 40, cols: 8, rows: 8)
        XCTAssertEqual(m.cell, GridMetrics.defaultMinimumCell)
    }

    func testGridToleratesZeroColumns() {
        let m = GridMetrics.fit(width: 200, height: 200, cols: 0, rows: -1)
        XCTAssertEqual(m.cols, 1)
        XCTAssertEqual(m.rows, 1)
        XCTAssertEqual(m.cell, 184, accuracy: 0.0001)
    }

    func testKeysAreRowMajor() {
        let m = GridMetrics.fit(width: 500, height: 300, cols: 4, rows: 2)
        XCTAssertEqual(m.keyIndex(row: 0, col: 0), 0)
        XCTAssertEqual(m.keyIndex(row: 0, col: 3), 3)
        XCTAssertEqual(m.keyIndex(row: 1, col: 0), 4)
        XCTAssertEqual(m.keyIndex(row: 1, col: 3), 7)
    }

    // MARK: Sliders

    func testSliderValueFromTouch() {
        XCTAssertEqual(SliderMath.value(atY: 0, height: 100), 100)
        XCTAssertEqual(SliderMath.value(atY: 100, height: 100), 0)
        XCTAssertEqual(SliderMath.value(atY: 50, height: 200), 75)
        XCTAssertEqual(SliderMath.value(atY: 0.5, height: 100), 99, "truncates like Android")
        XCTAssertEqual(SliderMath.value(atY: -40, height: 100), 100)
        XCTAssertEqual(SliderMath.value(atY: 400, height: 100), 0)
        XCTAssertEqual(SliderMath.value(atY: -1e300, height: 100), 100, "no overflow")
        XCTAssertEqual(SliderMath.value(atY: 10, height: 0), 0)
        XCTAssertEqual(SliderMath.value(atY: .nan, height: 100), 0)
    }

    func testSliderTicks() {
        XCTAssertTrue(SliderMath.crossesTick(from: 19, to: 20))
        XCTAssertFalse(SliderMath.crossesTick(from: 21, to: 29))
        XCTAssertTrue(SliderMath.crossesTick(from: 100, to: 99))
    }

    func testClampPercent() {
        XCTAssertEqual(clampPercent(-5), 0)
        XCTAssertEqual(clampPercent(55), 55)
        XCTAssertEqual(clampPercent(500), 100)
    }

    // MARK: Slider throttling

    func testThrottleSendsFirstMoveAtOnce() {
        var t = SliderThrottle()
        XCTAssertEqual(t.intervalMs, 50)
        XCTAssertEqual(t.change(index: 0, value: 10, final: false, nowMs: 1_000), .send(10))
    }

    func testThrottleHoldsFastMovesAndKeepsTheLatest() {
        var t = SliderThrottle(intervalMs: 50)
        XCTAssertEqual(t.change(index: 0, value: 10, final: false, nowMs: 1_000), .send(10))
        t.markSent(index: 0, nowMs: 1_000)
        XCTAssertEqual(t.change(index: 0, value: 20, final: false, nowMs: 1_010), .schedule(afterMs: 50))
        XCTAssertEqual(t.change(index: 0, value: 30, final: false, nowMs: 1_020), .coalesced)
        XCTAssertEqual(t.takePending(index: 0), 30)
        XCTAssertNil(t.takePending(index: 0))
        t.markSent(index: 0, nowMs: 1_060)
        // 50 ms after the last send it goes straight out again.
        XCTAssertEqual(t.change(index: 0, value: 40, final: false, nowMs: 1_110), .send(40))
    }

    func testThrottleAlwaysSendsTheFinalValue() {
        var t = SliderThrottle(intervalMs: 50)
        _ = t.change(index: 0, value: 10, final: false, nowMs: 0)
        t.markSent(index: 0, nowMs: 0)
        XCTAssertEqual(t.change(index: 0, value: 20, final: false, nowMs: 5), .schedule(afterMs: 50))
        XCTAssertEqual(t.change(index: 0, value: 25, final: true, nowMs: 6), .send(25))
        XCTAssertNil(t.takePending(index: 0), "the final send replaces the held value")
    }

    func testThrottleKeepsSlidersApartAndClamps() {
        var t = SliderThrottle(intervalMs: 50)
        XCTAssertEqual(t.change(index: 0, value: 10, final: false, nowMs: 0), .send(10))
        t.markSent(index: 0, nowMs: 0)
        XCTAssertEqual(t.change(index: 1, value: 150, final: false, nowMs: 1), .send(100))
        XCTAssertEqual(t.change(index: 0, value: -3, final: false, nowMs: 2), .schedule(afterMs: 50))
        XCTAssertEqual(t.takePending(index: 0), 0)
        t.reset()
        XCTAssertEqual(t.change(index: 0, value: 5, final: false, nowMs: 3), .send(5))
    }

    // MARK: Reconnecting

    func testBackoff() {
        XCTAssertEqual((0...7).map { Backoff.ms(attempt: $0) }, [500, 500, 1_000, 2_000, 4_000, 8_000, 8_000, 8_000])
    }

    func testScheduleWithOneAddress() {
        var s = ReconnectSchedule(urls: ["wss://a:1"])
        XCTAssertEqual(s.next(), "wss://a:1")
        XCTAssertEqual(s.ended(wasConnected: false), 500)
        XCTAssertEqual(s.next(), "wss://a:1")
        XCTAssertEqual(s.ended(wasConnected: false), 1_000)
        XCTAssertEqual(s.next(), "wss://a:1")
        XCTAssertEqual(s.attempt, 3)
        // A session that got going resets the backoff.
        XCTAssertEqual(s.ended(wasConnected: true), 500)
        XCTAssertEqual(s.next(), "wss://a:1")
        XCTAssertEqual(s.attempt, 1)
    }

    func testScheduleAlternatesFallbacksAndRemembersTheOneThatWorked() {
        var s = ReconnectSchedule(urls: ["wss://wifi:1", "wss://wifi:1", "wss://usb:1"])
        XCTAssertEqual(s.urls, ["wss://wifi:1", "wss://usb:1"])
        XCTAssertEqual(s.next(), "wss://wifi:1")
        XCTAssertEqual(s.ended(wasConnected: false), 0, "try the other address straight away")
        XCTAssertEqual(s.next(), "wss://usb:1")
        XCTAssertEqual(s.ended(wasConnected: false), 500, "a whole round failed: back off")
        XCTAssertEqual(s.next(), "wss://wifi:1")
        XCTAssertEqual(s.ended(wasConnected: false), 0)
        XCTAssertEqual(s.next(), "wss://usb:1")
        XCTAssertEqual(s.ended(wasConnected: true), 500)
        XCTAssertEqual(s.next(), "wss://usb:1", "the address that worked goes first")
        XCTAssertEqual(s.ended(wasConnected: false), 0)
        XCTAssertEqual(s.next(), "wss://wifi:1")
    }
}
