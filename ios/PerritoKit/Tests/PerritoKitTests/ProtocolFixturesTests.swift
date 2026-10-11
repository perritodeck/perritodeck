import Foundation
import XCTest
@testable import PerritoKit

/// Decodes and re-encodes every golden file in protocol/fixtures, like the Rust and Kotlin suites.
final class ProtocolFixturesTests: XCTestCase {
    /// `PERRITODECK_FIXTURES` overrides; otherwise the repo's protocol/fixtures, found from this file.
    private var fixturesDir: URL {
        if let path = ProcessInfo.processInfo.environment["PERRITODECK_FIXTURES"] {
            return URL(fileURLWithPath: path)
        }
        // ios/PerritoKit/Tests/PerritoKitTests/<this file> → repo root
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        return url.appendingPathComponent("protocol/fixtures")
    }

    private func fixtureFiles() throws -> [URL] {
        let files = try FileManager.default
            .contentsOfDirectory(at: fixturesDir, includingPropertiesForKeys: nil)
            .filter { $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }
        XCTAssertFalse(files.isEmpty, "no fixtures found in \(fixturesDir.path)")
        return files
    }

    private func jsonObject(_ text: String) throws -> NSDictionary {
        let obj = try JSONSerialization.jsonObject(with: Data(text.utf8))
        return try XCTUnwrap(obj as? NSDictionary)
    }

    func testEveryFixtureRoundTrips() throws {
        var seen = Set<String>()
        for file in try fixtureFiles() {
            let text = try String(contentsOf: file, encoding: .utf8)
            let decoded = try Codec.decode(text)
            guard case .known(let envelope) = decoded else {
                XCTFail("\(file.lastPathComponent): expected a known message, got \(decoded)")
                continue
            }
            seen.insert(envelope.message.type)
            let original = try jsonObject(text)
            let reencoded = try jsonObject(Codec.encode(envelope))
            XCTAssertEqual(original, reencoded, "\(file.lastPathComponent) did not round-trip")
        }
        XCTAssertEqual(seen, Message.knownTypes, "every message type needs a fixture")
    }

    private func fixture(_ name: String) throws -> Envelope {
        let text = try String(contentsOf: fixturesDir.appendingPathComponent(name), encoding: .utf8)
        guard case .known(let envelope) = try Codec.decode(text) else {
            XCTFail("\(name) is not a known message")
            throw ProtocolError.malformed(name)
        }
        return envelope
    }

    func testFixtureContents() throws {
        XCTAssertEqual(
            try fixture("keyDown.json"),
            Envelope(.keyDown(KeyEvent(keyIndex: 3, clientTimestampMs: 1_760_169_600_123)), id: 42)
        )
        XCTAssertEqual(
            try fixture("welcome.json").message,
            .welcome(Welcome(hostName: "dev-laptop", hostVersion: "0.1.0", protocolVersion: 1, heartbeatIntervalMs: 2000))
        )
        XCTAssertEqual(try fixture("setImage-clear.json").message, .setImage(SetImage(keyIndex: 1, image: nil)))
        XCTAssertEqual(try fixture("showOk.json").message, .showOk(KeyFeedback(keyIndex: 5)))
        XCTAssertEqual(try fixture("sliderChange.json").message, .sliderChange(SliderValue(keyIndex: 3, value: 64)))
        XCTAssertEqual(try fixture("setValue.json").message, .setValue(SliderValue(keyIndex: 3, value: 40)))
        XCTAssertEqual(
            try fixture("error-notPaired.json").message,
            .error(ErrorMessage(code: .notPaired, message: "This device isn't paired with this computer. Pair it again."))
        )

        guard case .layout(let layout) = try fixture("layout-slider.json").message else {
            return XCTFail("layout-slider.json is not a layout")
        }
        XCTAssertEqual(layout.cols, 2)
        XCTAssertFalse(layout.keys[0].isSlider)
        XCTAssertTrue(layout.keys[1].isSlider)
        XCTAssertEqual(layout.keys[1].value, 30)

        guard case .hello(let hello) = try fixture("hello-paired.json").message else {
            return XCTFail("hello-paired.json is not a hello")
        }
        XCTAssertEqual(hello.platform, .android)
        XCTAssertEqual(hello.deviceSecret, "Zk3x9QmV2pLr8TnW5yHc1dBs7aGe4uJo0iKf6XqRzMs")
        XCTAssertNil(hello.requestedGrid)

        guard case .paired(let paired) = try fixture("paired.json").message else {
            return XCTFail("paired.json is not paired")
        }
        XCTAssertTrue(Fingerprint.isWellFormed(paired.hostId))
    }

    func testClientMessagesEncodeLikeTheFixtures() throws {
        // What this client sends: hello with the iOS capabilities, keys, sliders, heartbeats, pair.
        let hello = Hello(
            deviceId: "abc",
            deviceName: "iPad",
            appVersion: "0.1.0",
            platform: .ios,
            osVersion: "17.0",
            screen: Screen(widthPx: 2048, heightPx: 1536, densityDpi: 320),
            capabilities: Hello.clientCapabilities,
            deviceSecret: "s3cret"
        )
        let json = try jsonObject(Codec.encode(Envelope(.hello(hello), id: 1)))
        XCTAssertEqual(json["v"] as? Int, 1)
        XCTAssertEqual(json["type"] as? String, "hello")
        XCTAssertEqual(json["id"] as? Int, 1)
        let payload = try XCTUnwrap(json["payload"] as? NSDictionary)
        XCTAssertEqual(payload["platform"] as? String, "ios")
        XCTAssertEqual(payload["capabilities"] as? [String], ["keys", "haptics", "multiTouch", "images", "sliders"])
        XCTAssertNil(payload["requestedGrid"], "nil optionals are left out")
        XCTAssertNil(payload["token"], "nil optionals are left out")
        XCTAssertEqual(payload["deviceSecret"] as? String, "s3cret")

        let heartbeat = try jsonObject(Codec.encode(Envelope(.heartbeat(Heartbeat(seq: 7, sentAtMs: 1_760_169_602_000)))))
        XCTAssertNil(heartbeat["id"], "no id unless given")

        let image = Codec.encode(Envelope(.setImage(SetImage(keyIndex: 1, image: "data:image/png;base64,AA/+"))))
        XCTAssertTrue(image.contains("data:image/png;base64,AA/+"), "slashes are not escaped: \(image)")
        let cleared = try jsonObject(Codec.encode(Envelope(.setImage(SetImage(keyIndex: 1, image: nil)))))
        let clearedPayload = try XCTUnwrap(cleared["payload"] as? NSDictionary)
        XCTAssertTrue(clearedPayload["image"] is NSNull, "a cleared image is encoded as null")
    }

    func testUnknownTypesAreReportedNotRejected() throws {
        let d = try Codec.decode(#"{"v":1,"type":"dialRotate","id":9,"payload":{"ticks":2}}"#)
        XCTAssertEqual(d, .unknownType("dialRotate", id: 9))
    }

    func testOtherMajorVersionsAreRejected() {
        XCTAssertThrowsError(try Codec.decode(#"{"v":2,"type":"hello","payload":{}}"#)) { error in
            XCTAssertEqual(error as? ProtocolError, .unsupportedVersion(2))
        }
    }

    func testGarbageIsRejected() {
        for text in ["nope", "[]", #"{"type":"hello","payload":{}}"#, #"{"v":1,"payload":{}}"#, #"{"v":1,"type":"keyUp"}"#] {
            XCTAssertThrowsError(try Codec.decode(text), text) { error in
                guard case .malformed? = error as? ProtocolError else {
                    return XCTFail("\(text): expected malformed, got \(error)")
                }
            }
        }
    }

    func testBadPayloadIsRejected() {
        XCTAssertThrowsError(try Codec.decode(#"{"v":1,"type":"keyDown","payload":{"keyIndex":"x"}}"#)) { error in
            guard case .malformed? = error as? ProtocolError else {
                return XCTFail("expected malformed, got \(error)")
            }
        }
    }

    func testExtraFieldsAreTolerated() throws {
        let d = try Codec.decode(#"{"v":1,"type":"setTitle","payload":{"keyIndex":1,"title":"a","font":"x"},"extra":true}"#)
        XCTAssertEqual(d, .known(Envelope(.setTitle(SetTitle(keyIndex: 1, title: "a")))))
    }

    func testColors() {
        XCTAssertEqual(RGBColor(hex: "#1E1E2E"), RGBColor(red: 0x1E, green: 0x1E, blue: 0x2E))
        XCTAssertEqual(RGBColor(hex: "#fab387"), RGBColor(red: 0xFA, green: 0xB3, blue: 0x87))
        XCTAssertNil(RGBColor(hex: "1E1E2E"))
        XCTAssertNil(RGBColor(hex: "#1E1E2"))
        XCTAssertNil(RGBColor(hex: "#GGGGGG"))
        XCTAssertNil(RGBColor(hex: "#+1E1E2"))
    }
}
