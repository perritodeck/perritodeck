import CryptoKit
import Foundation
import XCTest
@testable import PerritoKit

final class PairingTests: XCTestCase {
    func testBase64URLRoundTrips() {
        var generator = SystemRandomNumberGenerator()
        for length in 0...70 {
            let bytes = Data((0..<length).map { _ in UInt8.random(in: 0...255, using: &generator) })
            let encoded = Base64URL.encode(bytes)
            XCTAssertFalse(encoded.contains("="))
            XCTAssertFalse(encoded.contains("+"))
            XCTAssertFalse(encoded.contains("/"))
            XCTAssertEqual(Base64URL.decode(encoded), bytes, "length \(length)")
        }
        XCTAssertEqual(Base64URL.encode(Data([0xFB, 0xFF])), "-_8")
        XCTAssertNil(Base64URL.decode("not base64!"))
    }

    func testFingerprintMatchesTheHost() {
        // Same vector as the host's identity tests and Android: SHA-256 of nothing.
        let fp = Fingerprint.of(certificateDER: Data())
        XCTAssertEqual(fp, "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU")
        XCTAssertEqual(fp.count, 43)
        XCTAssertTrue(Fingerprint.isWellFormed(fp))
        XCTAssertEqual(Fingerprint.display(fp), "E3B0 C442 98FC")
        XCTAssertEqual(Fingerprint.display("not base64!"), "?")
    }

    private let fp = "q1w2e3r4t5y6u7i8o9p0a1s2d3f4g5h6j7k8l9z0x1c"

    func testParsesTheHostsPairingLink() throws {
        // Exactly what host/crates/core/src/discovery.rs produces.
        let link = try XCTUnwrap(PairingLink.parse(
            "perritodeck://pair?v=1&n=Dev%20Laptop&a=192.168.1.20%3A46710&a=10.0.0.5%3A46710&fp=\(fp)&c=042913"
        ))
        XCTAssertEqual(link.hostName, "Dev Laptop")
        XCTAssertEqual(link.addresses, [HostPort(host: "192.168.1.20", port: 46710), HostPort(host: "10.0.0.5", port: 46710)])
        XCTAssertEqual(link.fingerprint, fp)
        XCTAssertEqual(link.code, "042913")
    }

    func testPairingLinkDefaultsAndIPv6() throws {
        let link = try XCTUnwrap(PairingLink.parse("perritodeck://pair?v=1&a=%5Bfe80%3A%3A1%5D%3A46710&fp=\(fp)&c=123456"))
        XCTAssertEqual(link.hostName, "your computer")
        XCTAssertEqual(link.addresses, [HostPort(host: "fe80::1", port: 46710)])
    }

    func testRejectsBrokenLinks() {
        let ok = "perritodeck://pair?v=1&n=x&a=1.2.3.4%3A1&fp=\(fp)&c=123456"
        XCTAssertEqual(PairingLink.parse(ok)?.code, "123456")
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "v=1", with: "v=2")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "c=123456", with: "c=12345")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "c=123456", with: "c=12345a")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "fp=\(fp)", with: "fp=short")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "fp=\(fp)", with: "fp=\(fp.dropLast())!")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "&a=1.2.3.4%3A1", with: "")))
        XCTAssertNil(PairingLink.parse(ok.replacingOccurrences(of: "&a=1.2.3.4%3A1", with: "&a=1.2.3.4")))
        XCTAssertNil(PairingLink.parse("https://evil.example/pair?v=1"))
        XCTAssertNil(PairingLink.parse(""))
    }

    func testHostPorts() {
        XCTAssertEqual(HostPort.parse("[fe80::1]:46710"), HostPort(host: "fe80::1", port: 46710))
        XCTAssertEqual(HostPort.parse("192.168.1.20:46710"), HostPort(host: "192.168.1.20", port: 46710))
        XCTAssertNil(HostPort.parse("1.2.3.4"))
        XCTAssertNil(HostPort.parse("1.2.3.4:0"))
        XCTAssertNil(HostPort.parse("1.2.3.4:65536"))
        XCTAssertNil(HostPort.parse(":46710"))
        XCTAssertNil(HostPort.parse("[]:46710"))
    }

    func testTypedAddresses() {
        XCTAssertEqual(HostPort.parseTyped(" 192.168.1.20 "), HostPort(host: "192.168.1.20", port: defaultPort))
        XCTAssertEqual(HostPort.parseTyped("192.168.1.20:9000"), HostPort(host: "192.168.1.20", port: 9000))
        XCTAssertNil(HostPort.parseTyped("   "))
    }

    func testURLs() {
        XCTAssertEqual(webSocketURLString(host: "192.168.1.20", port: 46710), "wss://192.168.1.20:46710")
        XCTAssertEqual(webSocketURLString(host: "fe80::1", port: 46710), "wss://[fe80::1]:46710")
        XCTAssertEqual(formatHost("[::1]"), "[::1]")
        XCTAssertEqual(HostPort(host: "::1", port: 1).display, "[::1]:1")
    }

    func testPairingCodes() {
        XCTAssertTrue(PairingLink.isPairingCode("042913"))
        XCTAssertFalse(PairingLink.isPairingCode("42913"))
        XCTAssertFalse(PairingLink.isPairingCode("٤٢٩١٣٠"), "only ASCII digits")
    }

    func testCapitalizesHostErrors() {
        XCTAssertEqual(Pairing.capitalizingFirst("wrong code"), "Wrong code")
        XCTAssertEqual(Pairing.capitalizingFirst(""), "")
    }

    func testTimeout() async {
        do {
            _ = try await withTimeout(seconds: 0.05) { () async throws -> Int in
                try await Task.sleep(nanoseconds: 5_000_000_000)
                return 1
            }
            XCTFail("expected a timeout")
        } catch {
            XCTAssertTrue(error is TimeoutError, "\(error)")
        }
        let quick = try? await withTimeout(seconds: 5) { () async throws -> Int in 7 }
        XCTAssertEqual(quick, 7)
    }
}
