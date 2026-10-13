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
import dev.perritodeck.client.protocol.SliderValue
import dev.perritodeck.client.protocol.Welcome
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import okhttp3.Dns
import okhttp3.OkHttpClient
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.security.cert.CertificateException
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
    fun slidersThrottleAndFollowTheHost() = runBlocking {
        val layout = Layout(1, 1, listOf(KeySpec(0, "Vol", "#000000", "#FFFFFF", control = "slider", value = 50)))
        var hostWs: WebSocket? = null
        server.enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
            hostWs = ws
            ws.send(Codec.encode(Envelope(Message.WelcomeMsg(Welcome("fake", "0", 1, 2000)))))
            ws.send(Codec.encode(Envelope(Message.LayoutMsg(layout))))
        }))
        val conn = DeckConnection(http, scope)
        conn.connect(url(), hello)
        nextReceived() // hello
        withTimeout(5_000) { conn.layout.first { it?.keys?.firstOrNull()?.isSlider == true } }

        // A quick drag: many moves, few messages, and the final value always arrives.
        for (v in 0..40) conn.sliderChange(0, v)
        conn.sliderChange(0, 77, final = true)
        val values = mutableListOf<Int>()
        while (true) {
            val env = received.poll(500, TimeUnit.MILLISECONDS) ?: break
            (env.msg as? Message.SliderChange)?.let { values += it.payload.value }
        }
        assertTrue("throttled: $values", values.size < 10)
        assertEquals(77, values.last())
        assertEquals(77, conn.layout.value!!.keys[0].value)

        // The host moves it (volume changed on the computer).
        hostWs!!.send(Codec.encode(Envelope(Message.SetValue(SliderValue(0, 12)))))
        withTimeout(5_000) { conn.layout.first { it?.keys?.get(0)?.value == 12 } }
        conn.disconnect()
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
    fun failuresAreExplainedInPlainWords() {
        val cable = "wss://127.0.0.1:46710"
        val lan = "wss://192.168.1.60:46710"
        val describe = DeckConnection.Companion::describeFailure
        // What a Fire 7 showed with the computer's app quit and adb reverse still in place.
        val reset = javax.net.ssl.SSLHandshakeException(
            "SSL handshake aborted: ssl=0x8eac8ec8: I/O error during system call, Connection reset by peer",
        )
        assertEquals(DeckConnection.NOT_RUNNING_ON_USB, describe(reset, cable))
        assertEquals(
            "The connection to 192.168.1.60 was cut. Is PerritoDeck running on the computer?",
            describe(reset, lan),
        )
        val refused = java.net.ConnectException("Failed to connect to /192.168.1.60:46710")
        assertEquals(DeckConnection.NOTHING_ON_USB, describe(refused, cable))
        assertEquals("Nothing answers at 192.168.1.60. Is PerritoDeck running on the computer?", describe(refused, lan))
        val noWifi = java.net.ConnectException("connect failed: ENETUNREACH (Network is unreachable)")
        assertEquals("Can't reach 192.168.1.60 from this network.", describe(noWifi, lan))
        assertTrue(describe(java.net.SocketTimeoutException("timeout"), lan).startsWith("192.168.1.60 didn't answer."))
        assertEquals(DeckConnection.PIN_MISMATCH, describe(IOException("pin", CertificateException("x")), lan))
        // Anything else keeps its own words, without native pointers.
        assertEquals("odd failure", describe(IllegalStateException("ssl=0xdeadbeef: odd failure"), lan))
    }

    @Test
    fun aRefusedCableDoesNotHideWhatTheNetworkAddressSaid() = runBlocking {
        // No cable (127.0.0.1 refused) and nothing at the network address either: after the
        // first pass, the network address's explanation is the one shown.
        val deadCable = ServerSocket(0).use { it.localPort }
        val deadLan = ServerSocket(0).use { it.localPort }
        val conn = DeckConnection(http, scope)
        conn.connect("ws://127.0.0.1:$deadCable", hello, fallbacks = listOf("ws://127.0.0.2:$deadLan"))
        // Attempt 4 comes after the cable was refused a second time.
        val shown = withTimeout(10_000) {
            conn.state.first { it is ConnState.Connecting && it.attempt >= 4 }
        } as ConnState.Connecting
        assertEquals("Nothing answers at 127.0.0.2. Is PerritoDeck running on the computer?", shown.lastError)
        conn.disconnect()
    }

    @Test
    fun fallsThroughADeadAddressWithoutReportingIt() = runBlocking {
        // 127.0.0.1 comes first on every connect, and without a USB cable it's refused.
        val deadPort = ServerSocket(0).use { it.localPort }
        server.enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
            ws.send(Codec.encode(Envelope(Message.WelcomeMsg(Welcome("fake", "0", 1, 2000)))))
        }))
        val conn = DeckConnection(http, scope)
        val errors = mutableListOf<String?>()
        val connected = async(start = CoroutineStart.UNDISPATCHED) {
            withTimeout(10_000) {
                conn.state.first { s ->
                    if (s is ConnState.Connecting) errors += s.lastError
                    s is ConnState.Connected
                }
            }
        }
        conn.connect("ws://127.0.0.1:$deadPort", hello, fallbacks = listOf(url()))
        assertEquals(url(), (connected.await() as ConnState.Connected).url)
        assertTrue("first pass errors shown: $errors", errors.all { it == null })
        conn.disconnect()
    }

    @Test
    fun anotherComputerOnTheCableIsSkippedNotFatal() = runBlocking {
        // Stands in for the TLS pin: these addresses present someone else's certificate.
        val wrongComputer = setOf(1, 2)
        val pinned = http.newBuilder().addInterceptor { chain ->
            if (chain.request().url.port in wrongComputer) {
                throw IOException("pin", CertificateException("someone else"))
            }
            chain.proceed(chain.request())
        }.build()
        server.enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
            ws.send(Codec.encode(Envelope(Message.WelcomeMsg(Welcome("fake", "0", 1, 2000)))))
        }))

        // The cable leads to a different computer: the network address still gets its turn.
        val conn = DeckConnection(http, scope)
        conn.connect("ws://127.0.0.1:1", hello, pinned, fallbacks = listOf(url()))
        val connected = withTimeout(5_000) { conn.state.first { it is ConnState.Connected || it is ConnState.Failed } }
        assertEquals(url(), (connected as ConnState.Connected).url)
        conn.disconnect()

        // The network address itself answering as someone else is still the end of it.
        conn.connect("ws://127.0.0.1:1", hello, pinned, fallbacks = listOf("ws://192.0.2.1:2"))
        val failed = withTimeout(5_000) { conn.state.first { it is ConnState.Failed } }
        assertEquals(DeckConnection.PIN_MISMATCH, (failed as ConnState.Failed).reason)
    }

    @Test
    fun aNewIdentityOnTheCableKeepsTryingAndSaysToPairAgain() = runBlocking {
        // PerritoDeck reinstalled on the computer the tablet is plugged into, whose network
        // address can't be reached: from the tablet that looks just like a second computer
        // on the cable, so it keeps trying, and the message says what to do after a reinstall.
        val pinned = http.newBuilder().addInterceptor { chain ->
            if (chain.request().url.host == "127.0.0.1") throw IOException("pin", CertificateException("new identity"))
            chain.proceed(chain.request())
        }.build()
        val deadLan = ServerSocket(0).use { it.localPort }
        val conn = DeckConnection(http, scope)
        conn.connect("ws://127.0.0.1:1", hello, pinned, fallbacks = listOf("ws://127.0.0.2:$deadLan"))
        // Even attempts go to the network address; from the 4th they show what the cable said.
        val shown = withTimeout(10_000) {
            conn.state.first { it is ConnState.Failed || it is ConnState.Connecting && it.attempt >= 4 && it.attempt % 2 == 0 }
        }
        assertTrue("still trying: $shown", shown is ConnState.Connecting)
        val reason = (shown as ConnState.Connecting).lastError
        assertEquals(DeckConnection.OTHER_COMPUTER_ON_USB, reason)
        assertTrue(reason!!.contains("pair again"))
        conn.disconnect()
    }

    /** A fake host on 127.0.0.1 that welcomes whoever says hello, [times] connections. */
    private fun welcomingHost(times: Int = 1): MockWebServer = MockWebServer().apply {
        repeat(times) {
            enqueue(MockResponse().withWebSocketUpgrade(fakeHost { ws ->
                ws.send(Codec.encode(Envelope(Message.WelcomeMsg(Welcome("fake", "0", 1, 2000)))))
            }))
        }
        start(InetAddress.getByName("127.0.0.1"), 0)
    }

    /** `lan.test` stands in for the computer's network address; it leads to 127.0.0.1. */
    private val viaNetwork = http.newBuilder().dns(object : Dns {
        override fun lookup(hostname: String): List<InetAddress> =
            if (hostname == "lan.test") listOf(InetAddress.getByName("127.0.0.1")) else Dns.SYSTEM.lookup(hostname)
    }).build()

    /** `lan.test` doesn't answer at all, as when a firewall drops the connection. */
    private val dropped = http.newBuilder().addInterceptor { chain ->
        if (chain.request().url.host == "lan.test") throw java.net.SocketTimeoutException("connect timed out")
        chain.proceed(chain.request())
    }.build()

    @Test
    fun overTheCableAfterTheNetworkFailedItTellsTheHost() = runBlocking<Unit> {
        val cable = welcomingHost(times = 2)
        val changes = mutableListOf<Boolean>()
        val conn = DeckConnection(http, scope, networkAnswers = { false })
        // The network address goes first here, so it times out before the cable is tried.
        conn.connect(
            "ws://lan.test:46710",
            hello,
            dropped,
            fallbacks = listOf("ws://127.0.0.1:${cable.port}"),
            onNetworkFailed = { changes += it },
        )
        // In over the cable, it looks at the network once more; still nothing, so it
        // connects again to tell the computer.
        assertEquals(hello, (nextReceived().msg as Message.HelloMsg).payload)
        val said = (nextReceived().msg as Message.HelloMsg).payload
        assertTrue("networkFailed in $said", said.networkFailed)
        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        assertEquals(listOf(true), changes)
        conn.disconnect()
        runCatching { cable.shutdown() }
    }

    @Test
    fun aComputerThatWasStillStartingIsNotAFirewall() = runBlocking<Unit> {
        // While PerritoDeck isn't running, a firewall drops connections to its closed
        // port too. By the time the cable gets in, the network answers.
        val cable = welcomingHost()
        val changes = mutableListOf<Boolean>()
        val conn = DeckConnection(http, scope, networkAnswers = { true })
        conn.connect(
            "ws://lan.test:46710",
            hello,
            dropped,
            fallbacks = listOf("ws://127.0.0.1:${cable.port}"),
            onNetworkFailed = { changes += it },
        )
        assertEquals(hello, (nextReceived().msg as Message.HelloMsg).payload)
        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        Thread.sleep(300)
        assertTrue(changes.isEmpty())
        assertTrue("connected only once", received.isEmpty())
        conn.disconnect()
        runCatching { cable.shutdown() }
    }

    @Test
    fun aRefusedNetworkAddressIsNotTheFirewall() = runBlocking<Unit> {
        // Refused is what an address with no PerritoDeck gives, say while it restarts.
        val cable = welcomingHost()
        val deadLan = ServerSocket(0).use { it.localPort }
        val changes = mutableListOf<Boolean>()
        val conn = DeckConnection(http, scope)
        conn.connect(
            "ws://lan.test:$deadLan",
            hello,
            viaNetwork,
            fallbacks = listOf("ws://127.0.0.1:${cable.port}"),
            onNetworkFailed = { changes += it },
        )
        assertEquals(hello, (nextReceived().msg as Message.HelloMsg).payload)
        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        assertTrue(changes.isEmpty())
        conn.disconnect()
        runCatching { cable.shutdown() }
    }

    @Test
    fun gettingThroughOverTheNetworkClearsIt() = runBlocking<Unit> {
        val net = welcomingHost()
        val changes = mutableListOf<Boolean>()
        val conn = DeckConnection(http, scope)
        conn.connect("ws://lan.test:${net.port}", hello, viaNetwork, networkFailed = true, onNetworkFailed = { changes += it })
        // Only a connection over the cable says so.
        assertEquals(hello, (nextReceived().msg as Message.HelloMsg).payload)
        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        assertEquals(listOf(false), changes)
        conn.disconnect()
        runCatching { net.shutdown() }
    }

    @Test
    fun aNetworkThatStartsAnsweringIsNoticedFromTheCable() = runBlocking<Unit> {
        // Someone fixed the firewall while the tablet was on the cable: it looks again now
        // and then, and connects again so the computer hears the network works.
        val cable = welcomingHost(times = 2)
        val answers = java.util.concurrent.atomic.AtomicBoolean(false)
        val changes = mutableListOf<Boolean>()
        val conn = DeckConnection(http, scope, networkAnswers = { answers.get() }, networkRecheckMs = 200)
        conn.connect(
            "ws://127.0.0.1:${cable.port}",
            hello,
            fallbacks = listOf("ws://lan.test:1"),
            networkFailed = true,
            onNetworkFailed = { changes += it },
        )
        assertTrue((nextReceived().msg as Message.HelloMsg).payload.networkFailed)
        withTimeout(5_000) { conn.state.first { it is ConnState.Connected } }
        Thread.sleep(500) // a look or two that finds nothing
        assertTrue(changes.isEmpty())
        answers.set(true)
        assertEquals(hello, (nextReceived().msg as Message.HelloMsg).payload)
        assertEquals(listOf(false), changes)
        conn.disconnect()
        runCatching { cable.shutdown() }
    }

    @Test
    fun onlyAConnectionWithNoAnswerLooksLikeAFirewall() {
        val blocked = DeckConnection.Companion::looksBlocked
        assertTrue(blocked(java.net.SocketTimeoutException("connect timed out")))
        // What Android says.
        assertTrue(
            blocked(
                java.net.SocketTimeoutException(
                    "failed to connect to /192.168.1.60 (port 46710) from /192.168.1.189 (port 41234) after 10000ms",
                ),
            ),
        )
        assertTrue(blocked(IOException("wrapped", java.net.SocketTimeoutException("Connect timed out"))))
        // Something answered: no PerritoDeck there, or it went quiet after connecting.
        assertFalse(blocked(java.net.ConnectException("Failed to connect to /192.168.1.60:46710")))
        assertFalse(blocked(java.net.SocketTimeoutException("timeout")))
        // Wi-Fi off or another network: no firewall involved.
        assertFalse(blocked(java.net.ConnectException("connect failed: ENETUNREACH (Network is unreachable)")))
        assertFalse(blocked(java.net.NoRouteToHostException("No route to host")))
        assertFalse(blocked(java.net.UnknownHostException("lan.test")))
        assertFalse(blocked(javax.net.ssl.SSLHandshakeException("Connection reset by peer")))
    }

    @Test
    fun backoffGrowsAndCaps() {
        assertEquals(500, DeckConnection.backoffMs(1))
        assertEquals(1_000, DeckConnection.backoffMs(2))
        assertEquals(8_000, DeckConnection.backoffMs(5))
        assertEquals(8_000, DeckConnection.backoffMs(50))
    }
}
