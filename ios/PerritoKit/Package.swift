// swift-tools-version: 5.9
// Everything in Perrito for iOS that isn't UI: the wire protocol, pairing links,
// certificate pinning, the connection state machine, discovery, storage and the
// small pieces of maths the deck screen uses. No third-party dependencies.
import PackageDescription

let package = Package(
    name: "PerritoKit",
    // macOS is listed only so `swift test` runs on a Mac without a simulator.
    platforms: [.iOS(.v16), .macOS(.v13)],
    products: [
        .library(name: "PerritoKit", targets: ["PerritoKit"]),
    ],
    targets: [
        .target(name: "PerritoKit"),
        .testTarget(name: "PerritoKitTests", dependencies: ["PerritoKit"]),
    ]
)
