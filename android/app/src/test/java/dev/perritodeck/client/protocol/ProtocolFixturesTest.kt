package dev.perritodeck.client.protocol

import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File

class ProtocolFixturesTest {
    private val fixturesDir: File =
        File(System.getProperty("perritodeck.fixtures") ?: "../../protocol/fixtures")

    @Test
    fun everyFixtureRoundTrips() {
        val files = fixturesDir.listFiles { f -> f.extension == "json" }.orEmpty()
        assertTrue("no fixtures found in ${fixturesDir.absolutePath}", files.isNotEmpty())
        val seen = mutableSetOf<String>()
        for (f in files.sortedBy { it.name }) {
            val text = f.readText()
            val original = Codec.json.parseToJsonElement(text) as JsonObject
            val decoded = Codec.decode(text)
            if (decoded !is Decoded.Known) fail("${f.name}: expected a known message, got $decoded")
            val env = (decoded as Decoded.Known).envelope
            seen += env.msg.type
            assertEquals("${f.name} did not round-trip", original, Codec.toJson(env))
        }
        assertEquals("every message type needs a fixture", Codec.knownTypes, seen)
    }

    @Test
    fun unknownTypesAreReportedNotRejected() {
        val d = Codec.decode("""{"v":1,"type":"dialRotate","id":9,"payload":{"ticks":2}}""")
        assertEquals(Decoded.UnknownType("dialRotate", 9), d)
    }

    @Test(expected = UnsupportedVersionException::class)
    fun otherMajorVersionsAreRejected() {
        Codec.decode("""{"v":2,"type":"hello","payload":{}}""")
    }

    @Test(expected = ProtocolException::class)
    fun garbageIsRejected() {
        Codec.decode("nope")
    }

    @Test(expected = ProtocolException::class)
    fun badPayloadIsRejected() {
        Codec.decode("""{"v":1,"type":"keyDown","payload":{"keyIndex":"x"}}""")
    }

    @Test
    fun extraFieldsAreTolerated() {
        val d = Codec.decode("""{"v":1,"type":"setTitle","payload":{"keyIndex":1,"title":"a","font":"x"}}""")
        assertEquals(Message.SetTitleMsg(SetTitle(1, "a")), (d as Decoded.Known).envelope.msg)
    }

    @Test
    fun colors() {
        assertEquals(0xFF1E1E2E.toInt(), parseColor("#1E1E2E"))
        assertNull(parseColor("1E1E2E"))
        assertNull(parseColor("#1E1E2"))
        assertNull(parseColor("#GGGGGG"))
    }
}
