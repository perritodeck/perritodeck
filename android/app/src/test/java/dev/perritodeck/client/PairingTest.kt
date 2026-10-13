package dev.perritodeck.client

import dev.perritodeck.client.data.HostPort
import dev.perritodeck.client.data.PairedHost
import dev.perritodeck.client.data.PairingLink
import dev.perritodeck.client.net.Base64Url
import dev.perritodeck.client.net.Fingerprint
import dev.perritodeck.client.net.Pairing
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.ConnectException
import java.net.InetAddress
import java.net.ServerSocket
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
    fun pairingTriesTheCableFirstButSavesTheNetworkAddress() {
        val usb = HostPort("127.0.0.1", 46710)
        val lan = HostPort("192.168.1.60", 46710)
        val wsl = HostPort("172.17.0.1", 46710)
        val link = PairingLink.parse("perritodeck://pair?v=1&n=PC&a=192.168.1.60%3A46710&a=172.17.0.1%3A46710&fp=$fp&c=042913")!!
        assertEquals(listOf(usb, lan, wsl), link.candidates)
        // Paired over the cable: reconnects add 127.0.0.1 by themselves, so save Wi-Fi's.
        assertEquals(lan, link.addressToSave(usb))
        assertEquals(wsl, link.addressToSave(wsl))

        // A host with adb lists 127.0.0.1 itself, last; it's still tried once, first.
        val withUsb = PairingLink.parse("perritodeck://pair?v=1&n=PC&a=192.168.1.60%3A46710&a=127.0.0.1%3A46710&fp=$fp&c=042913")!!
        assertEquals(listOf(usb, lan), withUsb.candidates)
        assertEquals(lan, withUsb.addressToSave(usb))

        // Wi-Fi off on the computer: 127.0.0.1 is all there is, and that's what's saved.
        val usbOnly = PairingLink.parse("perritodeck://pair?v=1&n=PC&a=127.0.0.1%3A46710&fp=$fp&c=042913")!!
        assertEquals(listOf(usb), usbOnly.candidates)
        assertEquals(usb, usbOnly.addressToSave(usb))
    }

    @Test
    fun reconnectsTryTheCableThenTheNewestAddress() {
        val host = PairedHost(fp, "PC", "192.168.1.60", 46710, "secret")
        assertEquals(listOf("wss://127.0.0.1:46710", "wss://192.168.1.60:46710"), host.urls())
        // Discovery saw the computer somewhere new: tried before the saved address.
        assertEquals(
            listOf("wss://127.0.0.1:46711", "wss://192.168.1.77:46711", "wss://192.168.1.60:46710"),
            host.urls(HostPort("192.168.1.77", 46711)),
        )
        // Paired over the cable with the computer's Wi-Fi off: 127.0.0.1 once.
        assertEquals(listOf("wss://127.0.0.1:46710"), host.copy(address = "127.0.0.1").urls())
        assertEquals("wss://[fd24::5]:46710", HostPort("fd24::5", 46710).url)
    }

    @Test
    fun pairsOverTheFirstAddressThatAnswersAsTheComputer() = runBlocking {
        val usb = HostPort("127.0.0.1", 46710)
        val impostor = HostPort("172.17.0.1", 46710)
        val dropped = HostPort("10.9.9.9", 46710)
        val lan = HostPort("192.168.1.60", 46710)
        val stopped = CompletableDeferred<Unit>()
        val found = withTimeout(2_000) {
            Pairing.firstReachable(listOf(usb, impostor, dropped, lan), fp) { addr ->
                when (addr) {
                    usb -> throw ConnectException("refused") // no USB cable
                    impostor -> "another computer's fingerprint"
                    dropped -> try { awaitCancellation() } finally { stopped.complete(Unit) } // a firewall
                    else -> { delay(50); fp }
                }
            }
        }
        assertEquals(lan, found)
        assertTrue("the probe that hung was stopped", stopped.isCompleted)

        assertNull(Pairing.firstReachable(listOf(usb, lan), fp) { throw ConnectException("refused") })
        assertNull(Pairing.firstReachable(listOf(impostor), fp) { "another computer's fingerprint" })
    }

    @Test
    fun cancellingAProbeStopsItAtOnce() = runBlocking {
        // Accepts connections (the OS does that) but never answers the TLS handshake.
        ServerSocket(0, 50, InetAddress.getLoopbackAddress()).use { silent ->
            val probe = launch(Dispatchers.Default) { runCatching { Pairing.probe("127.0.0.1", silent.localPort) } }
            delay(300)
            probe.cancel()
            // Without closing its socket the probe would sit out its 5 s read timeout.
            withTimeout(2_000) { probe.join() }
        }
    }

    @Test
    fun hostPorts() {
        assertEquals(HostPort("fe80::1", 46710), HostPort.parse("[fe80::1]:46710"))
        assertNull(HostPort.parse("1.2.3.4"))
        assertNull(HostPort.parse("1.2.3.4:0"))
        assertNull(HostPort.parse(":46710"))
    }
}
