import Foundation

// PerritoDeck wire protocol v1. Source of truth: protocol/schema/v1.schema.json.
// The golden files in protocol/fixtures are decoded and re-encoded by ProtocolFixturesTests.
// Mirrors android/.../protocol/Protocol.kt.

public let protocolVersion = 1
public let defaultPort = 46710

public struct Screen: Codable, Equatable, Sendable {
    public var widthPx: Int
    public var heightPx: Int
    public var densityDpi: Int

    public init(widthPx: Int, heightPx: Int, densityDpi: Int) {
        self.widthPx = widthPx
        self.heightPx = heightPx
        self.densityDpi = densityDpi
    }
}

public struct Grid: Codable, Equatable, Sendable {
    public var cols: Int
    public var rows: Int

    public init(cols: Int, rows: Int) {
        self.cols = cols
        self.rows = rows
    }
}

public enum Platform: String, Codable, Equatable, Sendable {
    case android
    case ios
    case web
    case other
}

public struct Hello: Codable, Equatable, Sendable {
    public var deviceId: String
    public var deviceName: String
    public var appVersion: String
    public var platform: Platform
    public var osVersion: String
    public var screen: Screen
    public var capabilities: [String]
    public var requestedGrid: Grid?
    /// Deprecated Phase 1 LAN token; ignored by current hosts.
    public var token: String?
    /// Secret issued by `paired`.
    public var deviceSecret: String?
    /// Sent by a tablet on a USB cable whose connections to the computer's network address
    /// time out (its firewall may keep tablets on Wi-Fi out). iPhones and iPads have no
    /// cable connection, so Perrito for iOS never sets it; nil is left out of the JSON.
    public var networkFailed: Bool?

    /// What this client can do; the same list the Android app advertises.
    public static let clientCapabilities = ["keys", "haptics", "multiTouch", "images", "sliders"]

    public init(
        deviceId: String,
        deviceName: String,
        appVersion: String,
        platform: Platform,
        osVersion: String,
        screen: Screen,
        capabilities: [String],
        requestedGrid: Grid? = nil,
        token: String? = nil,
        deviceSecret: String? = nil,
        networkFailed: Bool? = nil
    ) {
        self.deviceId = deviceId
        self.deviceName = deviceName
        self.appVersion = appVersion
        self.platform = platform
        self.osVersion = osVersion
        self.screen = screen
        self.capabilities = capabilities
        self.requestedGrid = requestedGrid
        self.token = token
        self.deviceSecret = deviceSecret
        self.networkFailed = networkFailed
    }
}

public struct PairRequest: Codable, Equatable, Sendable {
    public var code: String
    public var deviceId: String
    public var deviceName: String

    public init(code: String, deviceId: String, deviceName: String) {
        self.code = code
        self.deviceId = deviceId
        self.deviceName = deviceName
    }
}

public struct Paired: Codable, Equatable, Sendable {
    public var deviceSecret: String
    public var hostId: String
    public var hostName: String

    public init(deviceSecret: String, hostId: String, hostName: String) {
        self.deviceSecret = deviceSecret
        self.hostId = hostId
        self.hostName = hostName
    }
}

public struct Welcome: Codable, Equatable, Sendable {
    public var hostName: String
    public var hostVersion: String
    public var protocolVersion: Int
    public var heartbeatIntervalMs: Int

    public init(hostName: String, hostVersion: String, protocolVersion: Int, heartbeatIntervalMs: Int) {
        self.hostName = hostName
        self.hostVersion = hostVersion
        self.protocolVersion = protocolVersion
        self.heartbeatIntervalMs = heartbeatIntervalMs
    }
}

public struct KeySpec: Codable, Equatable, Sendable {
    public var index: Int
    public var title: String
    public var bgColor: String
    public var textColor: String
    /// "slider" for a slider; nil (or "key") for an ordinary key.
    public var control: String?
    /// A slider's position, 0–100.
    public var value: Int?

    public var isSlider: Bool { control == "slider" }

    public init(index: Int, title: String, bgColor: String, textColor: String, control: String? = nil, value: Int? = nil) {
        self.index = index
        self.title = title
        self.bgColor = bgColor
        self.textColor = textColor
        self.control = control
        self.value = value
    }
}

public struct Layout: Codable, Equatable, Sendable {
    public var cols: Int
    public var rows: Int
    public var keys: [KeySpec]

    public init(cols: Int, rows: Int, keys: [KeySpec]) {
        self.cols = cols
        self.rows = rows
        self.keys = keys
    }
}

public struct KeyEvent: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var clientTimestampMs: Int64

    public init(keyIndex: Int, clientTimestampMs: Int64) {
        self.keyIndex = keyIndex
        self.clientTimestampMs = clientTimestampMs
    }
}

public struct KeyAck: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var hostProcessingUs: Int64

    public init(keyIndex: Int, hostProcessingUs: Int64) {
        self.keyIndex = keyIndex
        self.hostProcessingUs = hostProcessingUs
    }
}

public struct Heartbeat: Codable, Equatable, Sendable {
    public var seq: Int
    public var sentAtMs: Int64

    public init(seq: Int, sentAtMs: Int64) {
        self.seq = seq
        self.sentAtMs = sentAtMs
    }
}

public struct SetTitle: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var title: String

    public init(keyIndex: Int, title: String) {
        self.keyIndex = keyIndex
        self.title = title
    }
}

/// `image` is a PNG/WebP data URL, or nil to clear. Always encoded, even when nil (as `null`).
public struct SetImage: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var image: String?

    public init(keyIndex: Int, image: String?) {
        self.keyIndex = keyIndex
        self.image = image
    }

    private enum CodingKeys: String, CodingKey {
        case keyIndex, image
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(keyIndex, forKey: .keyIndex)
        if let image {
            try c.encode(image, forKey: .image)
        } else {
            try c.encodeNil(forKey: .image)
        }
    }
}

/// Payload of showOk / showAlert.
public struct KeyFeedback: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var message: String?

    public init(keyIndex: Int, message: String? = nil) {
        self.keyIndex = keyIndex
        self.message = message
    }
}

/// Payload of sliderChange (client → host) and setValue (host → client).
public struct SliderValue: Codable, Equatable, Sendable {
    public var keyIndex: Int
    public var value: Int

    public init(keyIndex: Int, value: Int) {
        self.keyIndex = keyIndex
        self.value = value
    }
}

public enum ErrorCode: String, Codable, Equatable, Sendable {
    case unauthorized
    case unsupportedVersion
    case badMessage
    case protocolViolation
    case `internal`
    case notPaired
    case pairingFailed
}

public struct ErrorMessage: Codable, Equatable, Sendable {
    public var code: ErrorCode
    public var message: String

    public init(code: ErrorCode, message: String) {
        self.code = code
        self.message = message
    }
}

/// Every message type, mirroring the Rust `Message` enum and Kotlin's `Message`.
public enum Message: Equatable, Sendable {
    case hello(Hello)
    case pair(PairRequest)
    case paired(Paired)
    case welcome(Welcome)
    case layout(Layout)
    case keyDown(KeyEvent)
    case keyUp(KeyEvent)
    case keyAck(KeyAck)
    case heartbeat(Heartbeat)
    case heartbeatAck(Heartbeat)
    case setTitle(SetTitle)
    case setImage(SetImage)
    case showOk(KeyFeedback)
    case showAlert(KeyFeedback)
    case sliderChange(SliderValue)
    case setValue(SliderValue)
    case error(ErrorMessage)

    public var type: String {
        switch self {
        case .hello: return "hello"
        case .pair: return "pair"
        case .paired: return "paired"
        case .welcome: return "welcome"
        case .layout: return "layout"
        case .keyDown: return "keyDown"
        case .keyUp: return "keyUp"
        case .keyAck: return "keyAck"
        case .heartbeat: return "heartbeat"
        case .heartbeatAck: return "heartbeatAck"
        case .setTitle: return "setTitle"
        case .setImage: return "setImage"
        case .showOk: return "showOk"
        case .showAlert: return "showAlert"
        case .sliderChange: return "sliderChange"
        case .setValue: return "setValue"
        case .error: return "error"
        }
    }

    public static let knownTypes: Set<String> = [
        "hello", "pair", "paired", "welcome", "layout", "keyDown", "keyUp", "keyAck",
        "heartbeat", "heartbeatAck", "setTitle", "setImage", "showOk", "showAlert",
        "sliderChange", "setValue", "error",
    ]
}

public struct Envelope: Equatable, Sendable {
    public var message: Message
    public var id: Int?
    public var v: Int

    public init(_ message: Message, id: Int? = nil, v: Int = protocolVersion) {
        self.message = message
        self.id = id
        self.v = v
    }
}

public enum Decoded: Equatable, Sendable {
    case known(Envelope)
    /// A well-formed envelope whose type this version doesn't know. Must be ignored.
    case unknownType(String, id: Int?)
}

public enum ProtocolError: Error, Equatable {
    /// Anything malformed: not JSON, missing `v`/`type`/`payload`, or a payload that doesn't fit.
    case malformed(String)
    /// A different major version; the session can't continue.
    case unsupportedVersion(Int)
}

public enum Codec {
    /// Encode one message as a JSON text frame. Optional fields that are nil are left out
    /// (except `setImage.image`, which is always present).
    public static func encode(_ envelope: Envelope) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        guard let data = try? encoder.encode(EncodableEnvelope(envelope)),
              let text = String(data: data, encoding: .utf8)
        else {
            // Can't happen for these types (only strings and integers), but never crash on the send path.
            return "{}"
        }
        return text
    }

    /// Decode one text frame. Unknown fields are tolerated; unknown types come back as `.unknownType`.
    public static func decode(_ text: String) throws -> Decoded {
        let data = Data(text.utf8)
        let decoder = JSONDecoder()
        let header: Header
        do {
            header = try decoder.decode(Header.self, from: data)
        } catch {
            throw ProtocolError.malformed("frame is not a JSON object")
        }
        guard let v = header.v else { throw ProtocolError.malformed("missing numeric `v`") }
        guard v == protocolVersion else { throw ProtocolError.unsupportedVersion(v) }
        guard let type = header.type else { throw ProtocolError.malformed("missing string `type`") }
        guard Message.knownTypes.contains(type) else { return .unknownType(type, id: header.id) }
        guard header.hasPayloadObject else { throw ProtocolError.malformed("`\(type)` has no payload object") }

        func p<T: Decodable>(_: T.Type) throws -> T {
            do {
                return try decoder.decode(PayloadBox<T>.self, from: data).payload
            } catch {
                throw ProtocolError.malformed("invalid `\(type)` message: \(error)")
            }
        }

        let message: Message
        switch type {
        case "hello": message = .hello(try p(Hello.self))
        case "pair": message = .pair(try p(PairRequest.self))
        case "paired": message = .paired(try p(Paired.self))
        case "welcome": message = .welcome(try p(Welcome.self))
        case "layout": message = .layout(try p(Layout.self))
        case "keyDown": message = .keyDown(try p(KeyEvent.self))
        case "keyUp": message = .keyUp(try p(KeyEvent.self))
        case "keyAck": message = .keyAck(try p(KeyAck.self))
        case "heartbeat": message = .heartbeat(try p(Heartbeat.self))
        case "heartbeatAck": message = .heartbeatAck(try p(Heartbeat.self))
        case "setTitle": message = .setTitle(try p(SetTitle.self))
        case "setImage": message = .setImage(try p(SetImage.self))
        case "showOk": message = .showOk(try p(KeyFeedback.self))
        case "showAlert": message = .showAlert(try p(KeyFeedback.self))
        case "sliderChange": message = .sliderChange(try p(SliderValue.self))
        case "setValue": message = .setValue(try p(SliderValue.self))
        case "error": message = .error(try p(ErrorMessage.self))
        default: return .unknownType(type, id: header.id)
        }
        return .known(Envelope(message, id: header.id, v: v))
    }

    // MARK: - Internals

    private enum EnvelopeKeys: String, CodingKey {
        case v, type, id, payload
    }

    /// Reads the envelope fields leniently so each problem gets the right error.
    private struct Header: Decodable {
        let v: Int?
        let type: String?
        let id: Int?
        let hasPayloadObject: Bool

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: EnvelopeKeys.self)
            v = try? c.decode(Int.self, forKey: .v)
            type = try? c.decode(String.self, forKey: .type)
            id = try? c.decode(Int.self, forKey: .id)
            hasPayloadObject = (try? c.nestedContainer(keyedBy: AnyKey.self, forKey: .payload)) != nil
        }
    }

    private struct AnyKey: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }

    private struct PayloadBox<T: Decodable>: Decodable {
        let payload: T
    }

    private struct EncodableEnvelope: Encodable {
        let envelope: Envelope

        init(_ envelope: Envelope) {
            self.envelope = envelope
        }

        func encode(to encoder: Encoder) throws {
            var c = encoder.container(keyedBy: EnvelopeKeys.self)
            try c.encode(envelope.v, forKey: .v)
            try c.encode(envelope.message.type, forKey: .type)
            try c.encodeIfPresent(envelope.id, forKey: .id)
            switch envelope.message {
            case .hello(let p): try c.encode(p, forKey: .payload)
            case .pair(let p): try c.encode(p, forKey: .payload)
            case .paired(let p): try c.encode(p, forKey: .payload)
            case .welcome(let p): try c.encode(p, forKey: .payload)
            case .layout(let p): try c.encode(p, forKey: .payload)
            case .keyDown(let p): try c.encode(p, forKey: .payload)
            case .keyUp(let p): try c.encode(p, forKey: .payload)
            case .keyAck(let p): try c.encode(p, forKey: .payload)
            case .heartbeat(let p): try c.encode(p, forKey: .payload)
            case .heartbeatAck(let p): try c.encode(p, forKey: .payload)
            case .setTitle(let p): try c.encode(p, forKey: .payload)
            case .setImage(let p): try c.encode(p, forKey: .payload)
            case .showOk(let p): try c.encode(p, forKey: .payload)
            case .showAlert(let p): try c.encode(p, forKey: .payload)
            case .sliderChange(let p): try c.encode(p, forKey: .payload)
            case .setValue(let p): try c.encode(p, forKey: .payload)
            case .error(let p): try c.encode(p, forKey: .payload)
            }
        }
    }
}

/// An opaque colour parsed from `#RRGGBB`; components are 0–255.
public struct RGBColor: Equatable, Sendable {
    public var red: Int
    public var green: Int
    public var blue: Int

    public init(red: Int, green: Int, blue: Int) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    /// Parse `#RRGGBB`, or nil if malformed.
    public init?(hex: String) {
        guard hex.count == 7, hex.first == "#" else { return nil }
        let digits = hex.dropFirst()
        guard digits.allSatisfy({ $0.isHexDigit && $0.isASCII }), let rgb = Int(digits, radix: 16) else { return nil }
        self.init(red: (rgb >> 16) & 0xFF, green: (rgb >> 8) & 0xFF, blue: rgb & 0xFF)
    }
}
