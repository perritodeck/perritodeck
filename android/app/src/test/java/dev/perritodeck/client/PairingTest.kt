package dev.perritodeck.client

import dev.perritodeck.client.data.HostPort
import dev.perritodeck.client.data.PairingLink
import dev.perritodeck.client.net.Base64Url
import dev.perritodeck.client.net.Fingerprint
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.util.Base64
import kotlin.random.Random

class PairingTest {
    @Test
    fun base64UrlMatchesTheJdk() {
        val rnd = Random(42)
        for (len in 0..70) {
            val bytes = rnd.nextBytes(len)
            val ours = Base64Url.encode(bytes)
            assertEquals(Base64.getUrlEncoder().withoutPadding().encodeToString(bytes), ours)
            assertArrayEquals(bytes, Base64Url.decode(ours))
        }
        assertNull(Base64Url.decode("not base64!"))
    }

    @Test
    fun fingerprintMatchesTheHost() {
        // Same vector as the host's identity tests: SHA-256 of nothing.
        val fp = Fingerprint.of(ByteArray(0))
        assertEquals(43, fp.length)
        assertEquals("E3B0 C442 98FC", Fingerprint.display(fp))
    }

    private val fp = "q1w2e3r4t5y6u7i8o9p0a1s2d3f4g5h6j7k8l9z0x1c"

    @Test
    fun parsesTheHostsPairingLink() {
        // Exactly what host/crates/core/src/discovery.rs produces.
        val link = PairingLink.parse(
            "perritodeck://pair?v=1&n=Dev%20Laptop&a=192.168.1.20%3A46710&a=10.0.0.5%3A46710&fp=$fp&c=042913"
        )!!
        assertEquals("Dev Laptop", link.hostName)
        assertEquals(listOf(HostPort("192.168.1.20", 46710), HostPort("10.0.0.5", 46710)), link.addresses)
        assertEquals(fp, link.fingerprint)
        assertEquals("042913", link.code)
    }

    @Test
    fun rejectsBrokenLinks() {
        val ok = "perritodeck://pair?v=1&n=x&a=1.2.3.4%3A1&fp=$fp&c=123456"
        assertEquals("123456", PairingLink.parse(ok)?.code)
        assertNull(PairingLink.parse(ok.replace("v=1", "v=2")))
        assertNull(PairingLink.parse(ok.replace("c=123456", "c=12345")))
        assertNull(PairingLink.parse(ok.replace("fp=$fp", "fp=short")))
        assertNull(PairingLink.parse(ok.replace("&a=1.2.3.4%3A1", "")))
        assertNull(PairingLink.parse("https://evil.example/pair?v=1"))
    }

    @Test
    fun hostPorts() {
        assertEquals(HostPort("fe80::1", 46710), HostPort.parse("[fe80::1]:46710"))
        assertNull(HostPort.parse("1.2.3.4"))
        assertNull(HostPort.parse("1.2.3.4:0"))
        assertNull(HostPort.parse(":46710"))
    }
}
