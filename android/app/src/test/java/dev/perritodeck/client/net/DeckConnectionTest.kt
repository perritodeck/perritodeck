package dev.perritodeck.client.net

import dev.perritodeck.client.protocol.Codec
import dev.perritodeck.client.protocol.Decoded
import dev.perritodeck.client.protocol.Envelope
import dev.perritodeck.client.protocol.ErrorCode
import dev.perritodeck.client.protocol.ErrorMessage
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.KeyAck
import dev.perritodeck.client.protocol.KeySpec
import dev.perritodeck.client.protocol.Layout
import dev.perritodeck.client.protocol.Message
import dev.perritodeck.client.protocol.Platform
import dev.perritodeck.client.protocol.Screen
import dev.perritodeck.client.protocol.SetTitle
import dev.perritodeck.client.protocol.Welcome
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.OkHttpClient
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/** Drives DeckConnection against a scripted fake host. */
class DeckConnectionTest {
    private lateinit var server: MockWebServer
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val http = OkHttpClient()
    private val received = LinkedBlockingQueue<Envelope>()

    private val hello = Hello(
        deviceId = "test-device",
        deviceName = "JUnit",
        appVersion = "0.0.0",
        platform = Platform.ANDROID,
        osVersion = "test",
        screen = Screen(1920, 1200, 224),
        capabilities = listOf("keys"),
    )

    @Before
    fun setUp() {
        server = MockWebServer()
        server.start()
    }

    @After
    fun tearDown() {
        scope.cancel()
        // A socket still closing can make shutdown time out; that isn't what these tests check.
        runCatching { server.shutdown() }
        http.dispatcher.executorService.shutdown()
    }

    /** A fake host that answers hello with [onHello] and acks every key event. */
    private fun fakeHost(onHello: (WebSocket) -> Unit) = object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            val env = (Codec.decode(text) as Decoded.Known).envelope
            received.add(env)
            when (val m = env.msg) {
                is Message.HelloMsg -> onHello(webSocket)
                is Message.KeyDown -> webSocket.send(Codec.encode(Envelope(Message.KeyAckMsg(KeyAck(m.payload.keyIndex, 5)), env.id)))
                is Message.KeyUp -> webSocket.send(Codec.encode(Envelope(Message.KeyAckMsg(KeyAck(m.payload.keyIndex, 5)), env.id)))
                else -> Unit
            }
        }
        // Answer the client's close so MockWebServer can shut down promptly.
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(1000, null)
        }
        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = Unit
    }

    private fun url() = "ws://${server.hostName}:${server.port}"

    private fun nextReceived(): Envelope =
        received.poll(5, TimeUnit.SECONDS) ?: throw AssertionError("host received nothing")

    @Test
    fun handshakeLayoutTitlesAndKeyAcks() = runBlocking {
        val layout = Layout(2, 1, listOf(KeySpec(0, "A", "#000000", "#FFFFFF"), KeySpec(1, "B", "#111111", "#FFFFFF")))
        server.enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
            ws.send(Codec.encode(Envelope(Message.WelcomeMsg(Welcome("fake", "0", 1, 2000)))))
            ws.send(Codec.encode(Envelope(Message.LayoutMsg(layout))))
            ws.send(Codec.encode(Envelope(Message.SetTitleMsg(SetTitle(1, "B2")))))
        }))

        val conn = DeckConnection(http, scope)
        conn.connect(url(), hello)

        val first = nextReceived()
        assertEquals(hello, (first.msg as Message.HelloMsg).payload)

        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        val titled = withTimeout(5_000) { conn.layout.first { it?.keys?.getOrNull(1)?.title == "B2" } }
        assertEquals("A", titled!!.keys[0].title)

        conn.keyDown(1)
        val down = nextReceived()
        assertTrue(down.msg is Message.KeyDown)
        assertEquals(1, (down.msg as Message.KeyDown).payload.keyIndex)
        val rtt = withTimeout(5_000) { conn.rttMs.first { it != null } }
        assertTrue("rtt should be measured", rtt!! >= 0.0)

        conn.disconnect()
        assertEquals(ConnState.Idle, conn.state.value)
    }

    @Test
    fun unauthorizedIsFatal() = runBlocking {
        server.enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
            ws.send(Codec.encode(Envelope(Message.Error(ErrorMessage(ErrorCode.UNAUTHORIZED, "wrong token")))))
            ws.close(1000, null)
        }))

        val conn = DeckConnection(http, scope)
        conn.connect(url(), hello)
        val failed = withTimeout(5_000) { conn.state.first { it is ConnState.Failed } }
        assertEquals("wrong token", (failed as ConnState.Failed).reason)
    }

    @Test
    fun unreachableHostKeepsRetrying() = runBlocking {
        val deadPort = server.port
        server.shutdown()
        val conn = DeckConnection(http, scope)
        conn.connect("ws://127.0.0.1:$deadPort", hello)
        val retrying = withTimeout(5_000) {
            conn.state.first { it is ConnState.Connecting && it.attempt >= 2 }
        }
        assertTrue((retrying as ConnState.Connecting).lastError != null)
        conn.disconnect()
    }

    @Test
    fun backoffGrowsAndCaps() {
        assertEquals(500, DeckConnection.backoffMs(1))
        assertEquals(1_000, DeckConnection.backoffMs(2))
        assertEquals(8_000, DeckConnection.backoffMs(5))
        assertEquals(8_000, DeckConnection.backoffMs(50))
    }
}
