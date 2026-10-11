package dev.perritodeck.client.protocol

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull

/**
 * PerritoDeck wire protocol v1. Source of truth: protocol/schema/v1.schema.json.
 * The golden files in protocol/fixtures are decoded and re-encoded by ProtocolFixturesTest.
 */
const val PROTOCOL_VERSION = 1
const val DEFAULT_PORT = 46710

@Serializable
data class Screen(val widthPx: Int, val heightPx: Int, val densityDpi: Int)

@Serializable
data class Grid(val cols: Int, val rows: Int)

@Serializable
enum class Platform {
    @SerialName("android") ANDROID,
    @SerialName("web") WEB,
    @SerialName("other") OTHER,
}

@Serializable
data class Hello(
    val deviceId: String,
    val deviceName: String,
    val appVersion: String,
    val platform: Platform,
    val osVersion: String,
    val screen: Screen,
    val capabilities: List<String>,
    val requestedGrid: Grid? = null,
    /** Deprecated Phase 1 LAN token; ignored by current hosts. */
    val token: String? = null,
    /** Secret issued by `paired`. */
    val deviceSecret: String? = null,
)

@Serializable
data class PairRequest(val code: String, val deviceId: String, val deviceName: String)

@Serializable
data class Paired(val deviceSecret: String, val hostId: String, val hostName: String)

@Serializable
data class Welcome(
    val hostName: String,
    val hostVersion: String,
    val protocolVersion: Int,
    val heartbeatIntervalMs: Long,
)

@Serializable
data class KeySpec(val index: Int, val title: String, val bgColor: String, val textColor: String)

@Serializable
data class Layout(val cols: Int, val rows: Int, val keys: List<KeySpec>)

@Serializable
data class KeyEvent(val keyIndex: Int, val clientTimestampMs: Long)

@Serializable
data class KeyAck(val keyIndex: Int, val hostProcessingUs: Long)

@Serializable
data class Heartbeat(val seq: Long, val sentAtMs: Long)

@Serializable
data class SetTitle(val keyIndex: Int, val title: String)

/** [image] is a PNG/WebP data URL, or null to clear. Always encoded, even when null. */
@Serializable
data class SetImage(val keyIndex: Int, val image: String?)

@Serializable
enum class ErrorCode {
    @SerialName("unauthorized") UNAUTHORIZED,
    @SerialName("unsupportedVersion") UNSUPPORTED_VERSION,
    @SerialName("badMessage") BAD_MESSAGE,
    @SerialName("protocolViolation") PROTOCOL_VIOLATION,
    @SerialName("internal") INTERNAL,
    @SerialName("notPaired") NOT_PAIRED,
    @SerialName("pairingFailed") PAIRING_FAILED,
}

@Serializable
data class ErrorMessage(val code: ErrorCode, val message: String)

/** Every message type, mirroring the Rust `Message` enum. */
sealed interface Message {
    val type: String

    data class HelloMsg(val payload: Hello) : Message { override val type = "hello" }
    data class PairMsg(val payload: PairRequest) : Message { override val type = "pair" }
    data class PairedMsg(val payload: Paired) : Message { override val type = "paired" }
    data class WelcomeMsg(val payload: Welcome) : Message { override val type = "welcome" }
    data class LayoutMsg(val payload: Layout) : Message { override val type = "layout" }
    data class KeyDown(val payload: KeyEvent) : Message { override val type = "keyDown" }
    data class KeyUp(val payload: KeyEvent) : Message { override val type = "keyUp" }
    data class KeyAckMsg(val payload: KeyAck) : Message { override val type = "keyAck" }
    data class HeartbeatMsg(val payload: Heartbeat) : Message { override val type = "heartbeat" }
    data class HeartbeatAck(val payload: Heartbeat) : Message { override val type = "heartbeatAck" }
    data class SetTitleMsg(val payload: SetTitle) : Message { override val type = "setTitle" }
    data class SetImageMsg(val payload: SetImage) : Message { override val type = "setImage" }
    data class Error(val payload: ErrorMessage) : Message { override val type = "error" }
}

data class Envelope(val msg: Message, val id: Long? = null, val v: Int = PROTOCOL_VERSION)

sealed interface Decoded {
    data class Known(val envelope: Envelope) : Decoded
    /** A well-formed envelope whose type this version doesn't know. Must be ignored. */
    data class UnknownType(val type: String, val id: Long?) : Decoded
}

class ProtocolException(message: String) : Exception(message)
class UnsupportedVersionException(val version: Long) :
    Exception("unsupported protocol version $version (this client speaks $PROTOCOL_VERSION)")

object Codec {
    /** Unknown fields tolerated (forward compatible); default-valued optionals omitted on encode. */
    val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
        explicitNulls = true
    }

    val knownTypes = setOf(
        "hello", "pair", "paired", "welcome", "layout", "keyDown", "keyUp", "keyAck",
        "heartbeat", "heartbeatAck", "setTitle", "setImage", "error",
    )

    fun encode(env: Envelope): String = json.encodeToString(JsonObject.serializer(), toJson(env))

    fun toJson(env: Envelope): JsonObject = buildJsonObject {
        put("v", JsonPrimitive(env.v))
        put("type", JsonPrimitive(env.msg.type))
        env.id?.let { put("id", JsonPrimitive(it)) }
        put("payload", payloadJson(env.msg))
    }

    private fun payloadJson(m: Message): JsonElement = when (m) {
        is Message.HelloMsg -> json.encodeToJsonElement(Hello.serializer(), m.payload)
        is Message.PairMsg -> json.encodeToJsonElement(PairRequest.serializer(), m.payload)
        is Message.PairedMsg -> json.encodeToJsonElement(Paired.serializer(), m.payload)
        is Message.WelcomeMsg -> json.encodeToJsonElement(Welcome.serializer(), m.payload)
        is Message.LayoutMsg -> json.encodeToJsonElement(Layout.serializer(), m.payload)
        is Message.KeyDown -> json.encodeToJsonElement(KeyEvent.serializer(), m.payload)
        is Message.KeyUp -> json.encodeToJsonElement(KeyEvent.serializer(), m.payload)
        is Message.KeyAckMsg -> json.encodeToJsonElement(KeyAck.serializer(), m.payload)
        is Message.HeartbeatMsg -> json.encodeToJsonElement(Heartbeat.serializer(), m.payload)
        is Message.HeartbeatAck -> json.encodeToJsonElement(Heartbeat.serializer(), m.payload)
        is Message.SetTitleMsg -> json.encodeToJsonElement(SetTitle.serializer(), m.payload)
        is Message.SetImageMsg -> json.encodeToJsonElement(SetImage.serializer(), m.payload)
        is Message.Error -> json.encodeToJsonElement(ErrorMessage.serializer(), m.payload)
    }

    /**
     * Decode one text frame.
     * @throws UnsupportedVersionException for a different major version
     * @throws ProtocolException for anything malformed
     */
    fun decode(text: String): Decoded {
        val obj = try {
            json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            throw ProtocolException("frame is not a JSON object")
        }
        val v = (obj["v"] as? JsonPrimitive)?.longOrNull
            ?: throw ProtocolException("missing numeric `v`")
        if (v != PROTOCOL_VERSION.toLong()) throw UnsupportedVersionException(v)
        val type = (obj["type"] as? JsonPrimitive)?.takeIf { it.isString }?.contentOrNull
            ?: throw ProtocolException("missing string `type`")
        val id = (obj["id"] as? JsonPrimitive)?.longOrNull
        if (type !in knownTypes) return Decoded.UnknownType(type, id)
        val payload = obj["payload"] as? JsonObject
            ?: throw ProtocolException("`$type` has no payload object")
        val msg = try {
            when (type) {
                "hello" -> Message.HelloMsg(p(Hello.serializer(), payload))
                "pair" -> Message.PairMsg(p(PairRequest.serializer(), payload))
                "paired" -> Message.PairedMsg(p(Paired.serializer(), payload))
                "welcome" -> Message.WelcomeMsg(p(Welcome.serializer(), payload))
                "layout" -> Message.LayoutMsg(p(Layout.serializer(), payload))
                "keyDown" -> Message.KeyDown(p(KeyEvent.serializer(), payload))
                "keyUp" -> Message.KeyUp(p(KeyEvent.serializer(), payload))
                "keyAck" -> Message.KeyAckMsg(p(KeyAck.serializer(), payload))
                "heartbeat" -> Message.HeartbeatMsg(p(Heartbeat.serializer(), payload))
                "heartbeatAck" -> Message.HeartbeatAck(p(Heartbeat.serializer(), payload))
                "setTitle" -> Message.SetTitleMsg(p(SetTitle.serializer(), payload))
                "setImage" -> Message.SetImageMsg(p(SetImage.serializer(), payload))
                "error" -> Message.Error(p(ErrorMessage.serializer(), payload))
                else -> error("unreachable: $type")
            }
        } catch (e: SerializationException) {
            throw ProtocolException("invalid `$type` message: ${e.message}")
        } catch (e: IllegalArgumentException) {
            throw ProtocolException("invalid `$type` message: ${e.message}")
        }
        return Decoded.Known(Envelope(msg, id, v.toInt()))
    }

    private fun <T> p(serializer: KSerializer<T>, payload: JsonObject): T =
        json.decodeFromJsonElement(serializer, payload)
}

/** Parse `#RRGGBB` into an opaque ARGB int, or null if malformed. */
fun parseColor(hex: String): Int? {
    if (hex.length != 7 || hex[0] != '#') return null
    val rgb = hex.substring(1).toIntOrNull(16) ?: return null
    return (0xFF shl 24) or rgb
}
