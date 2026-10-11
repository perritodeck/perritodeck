import CoreGraphics
import Foundation
import XCTest
@testable import PerritoKit

final class SettingsTests: XCTestCase {
    private var suiteName = ""
    private var defaults: UserDefaults!

    override func setUp() {
        super.setUp()
        suiteName = "PerritoKitTests-\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        super.tearDown()
    }

    private func host(_ id: String, address: String = "192.168.1.20") -> PairedHost {
        PairedHost(hostId: id, name: "PC \(id)", address: address, port: 46710, deviceSecret: "secret-\(id)")
    }

    func testDeviceIdIsStable() {
        let store = MemorySecureStore()
        let settings = Settings(secure: store, defaults: defaults)
        let id = settings.deviceId
        XCTAssertFalse(id.isEmpty)
        XCTAssertEqual(settings.deviceId, id)
        XCTAssertEqual(Settings(secure: store, defaults: defaults).deviceId, id)
    }

    func testPairedHostsAreKeptInTheSecureStore() {
        let store = MemorySecureStore()
        let settings = Settings(secure: store, defaults: defaults)
        settings.savePaired(host("a"))
        settings.savePaired(host("b"))
        settings.savePaired(host("a", address: "10.0.0.5"))
        XCTAssertEqual(settings.pairedHosts.map(\.hostId), ["b", "a"])
        XCTAssertEqual(settings.pairedHosts.last?.address, "10.0.0.5")
        XCTAssertEqual(settings.lastHostId, "a")
        XCTAssertNotNil(store.data(for: "pairedHosts"))
        XCTAssertNil(defaults.data(forKey: "pairedHosts"), "secrets never go to UserDefaults")

        settings.updateAddress(hostId: "b", address: "192.168.1.99", port: 9000)
        XCTAssertEqual(settings.pairedHosts.first?.url, "wss://192.168.1.99:9000")

        settings.forget(hostId: "a")
        XCTAssertEqual(settings.pairedHosts.map(\.hostId), ["b"])
        XCTAssertNil(settings.lastHostId)
    }

    func testAReinstallStartsFresh() {
        let store = MemorySecureStore()
        let first = Settings(secure: store, defaults: defaults)
        let id = first.deviceId
        first.savePaired(host("a"))

        // Same Keychain, but the app's UserDefaults were removed with the app.
        defaults.removePersistentDomain(forName: suiteName)
        let again = Settings(secure: store, defaults: defaults)
        XCTAssertTrue(again.pairedHosts.isEmpty)
        XCTAssertNotEqual(again.deviceId, id)
    }

    func testManualAddressIsRemembered() {
        let settings = Settings(secure: MemorySecureStore(), defaults: defaults)
        XCTAssertEqual(settings.lastManualAddress, "")
        settings.lastManualAddress = "192.168.1.20:46710"
        XCTAssertEqual(Settings(secure: MemorySecureStore(), defaults: defaults).lastManualAddress, "192.168.1.20:46710")
    }
}

final class KeyImagesTests: XCTestCase {
    func testDecodesTheFixtureImage() throws {
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
        let image = try XCTUnwrap(KeyImages.decode(url))
        XCTAssertEqual(image.width, 1)
        XCTAssertEqual(image.height, 1)
    }

    func testRejectsOtherData() {
        XCTAssertNil(KeyImages.decode("data:image/gif;base64,R0lGODlhAQABAAAAACw="))
        XCTAssertNil(KeyImages.decode("https://example.com/key.png"))
        XCTAssertNil(KeyImages.decode("data:image/png;base64,bm90IGFuIGltYWdl"))
        XCTAssertNil(KeyImages.bytes(fromDataURL: "data:text/plain;base64,AAAA"))
        XCTAssertEqual(KeyImages.bytes(fromDataURL: "data:image/webp;base64,AAEC"), Data([0, 1, 2]))
    }
}
