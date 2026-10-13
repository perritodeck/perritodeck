package dev.perritodeck.client.net

import dev.perritodeck.client.data.isLoopback
import dev.perritodeck.client.protocol.Codec
import dev.perritodeck.client.protocol.Decoded
import dev.perritodeck.client.protocol.Envelope
import dev.perritodeck.client.protocol.ErrorCode
import dev.perritodeck.client.protocol.Heartbeat
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.KeyEvent
import dev.perritodeck.client.protocol.Layout
import dev.perritodeck.client.protocol.Message
import dev.perritodeck.client.protocol.SliderValue
import dev.perritodeck.client.protocol.ProtocolException
import dev.perritodeck.client.protocol.UnsupportedVersionException
import dev.perritodeck.client.protocol.Welcome
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/** Whether something takes a TCP connection at [url]'s address and port within 3 s. */
private suspend fun tcpAnswers(url: String): Boolean = withContext(Dispatchers.IO) {
    val authority = url.substringAfter("://").substringBefore('/')
    val host = authority.substringBeforeLast(':').removeSurrounding("[", "]")
    val port = authority.substringAfterLast(':').toIntOrNull() ?: return@withContext false
    runCatching { Socket().use { it.connect(InetSocketAddress(host, port), 3_000) } }.isSuccess
}

sealed interface ConnState {
    data object Idle : ConnState
    data class Connecting(val url: String, val attempt: Int, val lastError: String?) : ConnState
    /** [url] is the address that worked. */
    data class Connected(val welcome: Welcome, val url: String) : ConnState
    /** The host refused us for a reason retrying won't fix (not paired, wrong version, wrong host). */
    data class Failed(val reason: String, val notPaired: Boolean = false) : ConnState
}

/**
 * One logical connection to a host: connects, performs the hello handshake, keeps the
 * layout up to date, sends key events, heartbeats, and reconnects with backoff.
 * All public functions are safe to call from the main thread.
 */
class DeckConnection(
    private val http: OkHttpClient,
    private val scope: CoroutineScope,
    private val nowMs: () -> Long = System::currentTimeMillis,
    private val nanoTime: () -> Long = System::nanoTime,
    /** Turns a `setImage` data URL into something drawable; Android bitmaps in the app, fakes in tests. */
    private val decodeImage: (String) -> Any? = KeyImages::decode,
    /** Whether the computer answers at a network address (a plain TCP connect). */
    private val networkAnswers: suspend (url: String) -> Boolean = ::tcpAnswers,
    /** How often to look again at the network while on the cable because it failed. */
    private val networkRecheckMs: Long = NETWORK_RECHECK_MS,
) {
    private val _state = MutableStateFlow<ConnState>(ConnState.Idle)
    val state: StateFlow<ConnState> = _state.asStateFlow()

    private val _layout = MutableStateFlow<Layout?>(null)
    /** Last layout received; kept while reconnecting so the grid stays on screen. */
    val layout: StateFlow<Layout?> = _layout.asStateFlow()

    private val _images = MutableStateFlow<Map<Int, Any>>(emptyMap())
    /** Host-rendered key images by key index (from `setImage`); cleared with each new layout. */
    val images: StateFlow<Map<Int, Any>> = _images.asStateFlow()

    /** A short-lived ✓ / ⚠ on a key after its action finished. */
    data class Flash(val keyIndex: Int, val ok: Boolean, val at: Long)

    private val _flash = MutableStateFlow<Flash?>(null)
    val flash: StateFlow<Flash?> = _flash.asStateFlow()

    private val _rttMs = MutableStateFlow<Double?>(null)
    /** Round trip of the most recent key press, in milliseconds. */
    val rttMs: StateFlow<Double?> = _rttMs.asStateFlow()

    private var job: Job? = null
    @Volatile private var socket: WebSocket? = null
    private val nextId = AtomicLong(1)
    private val pendingAcks = ConcurrentHashMap<Long, Long>()

    /**
     * Connect and stay connected. [client] carries the TLS pin for this host; defaults to
     * the constructor's client. [fallbacks] are other addresses for the same host
     * (127.0.0.1 for a tablet on USB); retries alternate between them, and the one that
     * worked is tried first next time.
     *
     * [networkFailed] says the host's network addresses didn't answer last time; while
     * it's so, connections over the USB cable tell the host ([Hello.networkFailed]), so it
     * can point at its firewall. [onNetworkFailed] hears when that changes, to remember it.
     */
    fun connect(
        url: String,
        hello: Hello,
        client: OkHttpClient = http,
        fallbacks: List<String> = emptyList(),
        networkFailed: Boolean = false,
        onNetworkFailed: (Boolean) -> Unit = {},
    ) {
        disconnect()
        _layout.value = null
        _images.value = emptyMap()
        val urls = (listOf(url) + fallbacks).distinct()
        job = scope.launch { runLoop(urls, hello, client, networkFailed, onNetworkFailed) }
    }

    fun disconnect() {
        job?.cancel()
        job = null
        socket?.close(1000, "client disconnected")
        socket = null
        pendingAcks.clear()
        _state.value = ConnState.Idle
    }

    fun keyDown(index: Int) = sendKey(Message.KeyDown(KeyEvent(index, nowMs())))

    fun keyUp(index: Int) = sendKey(Message.KeyUp(KeyEvent(index, nowMs())))

    private val sliderPending = ConcurrentHashMap<Int, Int>()
    private val sliderLastSent = ConcurrentHashMap<Int, Long>()

    /**
     * A slider moved to [value] (0–100). Sent at most every [SLIDER_INTERVAL_MS] while
     * dragging (the latest value wins), and always when [final] (the finger lifted).
     */
    fun sliderChange(index: Int, value: Int, final: Boolean = false) {
        val v = value.coerceIn(0, 100)
        setSliderValue(index, v)
        val now = nanoTime()
        val last = sliderLastSent[index] ?: 0L
        if (final || now - last >= SLIDER_INTERVAL_MS * 1_000_000) {
            sliderPending.remove(index)
            sendSlider(index, v)
        } else if (sliderPending.put(index, v) == null) {
            scope.launch {
                delay(SLIDER_INTERVAL_MS)
                sliderPending.remove(index)?.let { sendSlider(index, it) }
            }
        }
    }

    private fun sendSlider(index: Int, value: Int) {
        val ws = socket ?: return
        if (_state.value !is ConnState.Connected) return
        sliderLastSent[index] = nanoTime()
        ws.send(Codec.encode(Envelope(Message.SliderChange(SliderValue(index, value)))))
    }

    private fun setSliderValue(index: Int, value: Int) {
        _layout.value = _layout.value?.let { l ->
            l.copy(keys = l.keys.map { k -> if (k.index == index && k.isSlider) k.copy(value = value) else k })
        }
    }

    private fun sendKey(msg: Message) {
        val ws = socket ?: return
        if (_state.value !is ConnState.Connected) return
        val id = nextId.getAndIncrement()
        if (pendingAcks.size > 256) pendingAcks.clear()
        pendingAcks[id] = nanoTime()
        ws.send(Codec.encode(Envelope(msg, id)))
    }

    private suspend fun runLoop(
        urls: List<String>,
        hello: Hello,
        client: OkHttpClient,
        networkFailedAtStart: Boolean,
        onNetworkFailed: (Boolean) -> Unit,
    ) {
        var attempt = 0
        var lastError: String? = null
        var first = 0 // index of the address that last worked
        var everConnected = false
        val networkFailed = AtomicBoolean(networkFailedAtStart)
        fun setNetworkFailed(failed: Boolean) {
            if (networkFailed.getAndSet(failed) != failed) onNetworkFailed(failed)
        }
        val networkUrls = urls.filterNot { isLoopback(hostOf(it)) }
        while (currentCoroutineContext().isActive) {
            attempt++
            val i = (first + attempt - 1) % urls.size
            val url = urls[i]
            val overCable = isLoopback(hostOf(url))
            // On the first pass one address failing isn't news while the next is tried
            // (127.0.0.1 is refused whenever there's no USB cable).
            val shownError = if (!everConnected && attempt <= urls.size) null else lastError
            _state.value = ConnState.Connecting(url, attempt, shownError)
            val sessionHello = if (overCable && networkFailed.get()) hello.copy(networkFailed = true) else hello
            var recheck: Job? = null
            var end = runSession(url, sessionHello, client) {
                if (!overCable) {
                    // Through over the network: whatever kept it out is fixed.
                    setNetworkFailed(false)
                } else if (sessionHello.networkFailed && networkUrls.isNotEmpty()) {
                    // On the cable because the network failed: look again now and then, and
                    // once it answers, connect again so the computer hears it's fixed.
                    recheck = scope.launch {
                        while (isActive) {
                            delay(networkRecheckMs)
                            if (networkUrls.any { networkAnswers(it) }) {
                                setNetworkFailed(false)
                                socket?.cancel()
                                return@launch
                            }
                        }
                    }
                }
            }
            recheck?.cancel()
            // The network address didn't answer at all: maybe the computer's firewall.
            if (end is SessionEnd.Retry && !end.wasConnected && end.blocked && !overCable) {
                setNetworkFailed(true)
            }
            // Over a USB cable 127.0.0.1 leads to whichever computer the tablet is plugged
            // into, which needn't be this one. That's no reason to give up on the others.
            // It can also be this one with a new identity (PerritoDeck reinstalled, its data
            // folder wiped), which looks the same from here, so the message covers both.
            // Giving up instead would strand a tablet plugged into a second computer for
            // good while this one sleeps.
            if (end is SessionEnd.Fatal && end.reason == PIN_MISMATCH && urls.size > 1 && isLoopback(hostOf(url))) {
                end = SessionEnd.Retry(OTHER_COMPUTER_ON_USB, false)
            }
            when (end) {
                is SessionEnd.Fatal -> {
                    _state.value = ConnState.Failed(end.reason, end.notPaired)
                    return
                }
                is SessionEnd.Retry -> {
                    // 127.0.0.1 is refused whenever there's no cable, so with other addresses
                    // to try, what they said is the better explanation.
                    if (end.reason != NOTHING_ON_USB || urls.size == 1) lastError = end.reason
                    if (end.wasConnected) {
                        attempt = 0
                        first = i
                        everConnected = true
                    }
                }
            }
            // Try the other addresses straight away before backing off.
            if (urls.size > 1 && attempt % urls.size != 0) continue
            delay(backoffMs(attempt / urls.size.coerceAtLeast(1)))
        }
    }

    private sealed interface SessionEnd {
        /** [blocked]: connecting to the address got no answer ([looksBlocked]). */
        data class Retry(val reason: String, val wasConnected: Boolean, val blocked: Boolean = false) : SessionEnd
        data class Fatal(val reason: String, val notPaired: Boolean = false) : SessionEnd
    }

    private sealed interface WsEvent {
        data object Open : WsEvent
        data class Text(val text: String) : WsEvent
        data class Closed(val reason: String) : WsEvent
        data class Failure(val reason: String, val blocked: Boolean = false) : WsEvent
    }

    /** [onConnected] runs when the host welcomes this tablet. */
    private suspend fun runSession(
        url: String,
        hello: Hello,
        client: OkHttpClient,
        onConnected: () -> Unit = {},
    ): SessionEnd {
        val events = Channel<WsEvent>(Channel.UNLIMITED)
        val request = try {
            Request.Builder().url(url).build()
        } catch (e: IllegalArgumentException) {
            return SessionEnd.Fatal("Invalid address: $url")
        }
        val ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                events.trySend(WsEvent.Open)
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                events.trySend(WsEvent.Text(text))
            }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
                events.trySend(WsEvent.Closed(reason.ifEmpty { "host closed the connection" }))
            }
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                events.trySend(WsEvent.Closed(reason.ifEmpty { "connection closed" }))
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                events.trySend(WsEvent.Failure(describeFailure(t, url), looksBlocked(t)))
            }
        })
        socket = ws
        var connected = false
        var heartbeat: Job? = null
        val lastHeard = AtomicLong(nanoTime())
        try {
            when (val first = events.receive()) {
                WsEvent.Open -> ws.send(Codec.encode(Envelope(Message.HelloMsg(hello), nextId.getAndIncrement())))
                is WsEvent.Failure -> return if (first.reason == PIN_MISMATCH) {
                    SessionEnd.Fatal(first.reason)
                } else {
                    SessionEnd.Retry(first.reason, false, first.blocked)
                }
                is WsEvent.Closed -> return SessionEnd.Retry(first.reason, false)
                is WsEvent.Text -> return SessionEnd.Retry("unexpected data before open", false)
            }
            for (event in events) {
                when (event) {
                    WsEvent.Open -> Unit
                    is WsEvent.Failure -> return SessionEnd.Retry(event.reason, connected)
                    is WsEvent.Closed -> return SessionEnd.Retry(event.reason, connected)
                    is WsEvent.Text -> {
                        lastHeard.set(nanoTime())
                        val decoded = try {
                            Codec.decode(event.text)
                        } catch (e: UnsupportedVersionException) {
                            return SessionEnd.Fatal("The host speaks a newer protocol. Update Perrito.")
                        } catch (e: ProtocolException) {
                            continue // a single bad frame isn't worth dropping the session
                        }
                        val msg = (decoded as? Decoded.Known)?.envelope ?: continue
                        when (val m = msg.msg) {
                            is Message.WelcomeMsg -> {
                                connected = true
                                onConnected()
                                _state.value = ConnState.Connected(m.payload, url)
                                heartbeat?.cancel()
                                heartbeat = startHeartbeat(ws, m.payload.heartbeatIntervalMs, lastHeard)
                            }
                            is Message.LayoutMsg -> {
                                _layout.value = m.payload
                                _images.value = emptyMap()
                            }
                            is Message.ShowOk -> _flash.value = Flash(m.payload.keyIndex, true, nanoTime())
                            is Message.ShowAlert -> _flash.value = Flash(m.payload.keyIndex, false, nanoTime())
                            is Message.SetValue -> setSliderValue(m.payload.keyIndex, m.payload.value.coerceIn(0, 100))
                            is Message.SetImageMsg -> {
                                val key = m.payload.keyIndex
                                val decoded = m.payload.image?.let(decodeImage)
                                _images.value = if (decoded == null) _images.value - key else _images.value + (key to decoded)
                            }
                            is Message.SetTitleMsg -> _layout.value = _layout.value?.let { l ->
                                l.copy(keys = l.keys.map { k ->
                                    if (k.index == m.payload.keyIndex) k.copy(title = m.payload.title) else k
                                })
                            }
                            is Message.KeyAckMsg -> msg.id?.let { id ->
                                pendingAcks.remove(id)?.let { sent -> _rttMs.value = (nanoTime() - sent) / 1e6 }
                            }
                            is Message.Error -> when (m.payload.code) {
                                ErrorCode.UNAUTHORIZED -> return SessionEnd.Fatal(m.payload.message)
                                ErrorCode.NOT_PAIRED -> return SessionEnd.Fatal(
                                    "This computer no longer recognises this tablet. Pair it again.",
                                    notPaired = true,
                                )
                                ErrorCode.UNSUPPORTED_VERSION -> return SessionEnd.Fatal(m.payload.message)
                                else -> Unit // non-fatal; Phase 2 surfaces these in a debug panel
                            }
                            else -> Unit // heartbeatAck only refreshes lastHeard
                        }
                    }
                }
            }
            return SessionEnd.Retry("connection closed", connected)
        } finally {
            heartbeat?.cancel()
            events.close()
            ws.cancel()
            if (socket === ws) socket = null
            pendingAcks.clear()
        }
    }

    private fun startHeartbeat(ws: WebSocket, intervalMs: Long, lastHeard: AtomicLong): Job = scope.launch {
        val interval = intervalMs.coerceIn(250, 60_000)
        var seq = 0L
        while (isActive) {
            delay(interval)
            // No traffic for 3 intervals: the host is gone (Wi-Fi dropped, laptop slept). Force a reconnect.
            if ((nanoTime() - lastHeard.get()) / 1_000_000 > interval * 3) {
                ws.cancel()
                return@launch
            }
            seq++
            ws.send(Codec.encode(Envelope(Message.HeartbeatMsg(Heartbeat(seq, nowMs())))))
        }
    }

    companion object {
        const val OTHER_COMPUTER_ON_USB =
            "The computer on the USB cable isn't the one this tablet paired with. If you reinstalled PerritoDeck there, pair again."
        const val NOTHING_ON_USB = "Nothing answers over the USB cable."
        const val NOT_RUNNING_ON_USB = "PerritoDeck isn't running on the computer this tablet is plugged into."

        /**
         * What to tell the person when connecting to [url] failed with [t]. The exception's
         * own text is written for developers: Conscrypt's reads "SSL handshake aborted:
         * ssl=0x8eac8ec8: I/O error during system call, Connection reset by peer".
         */
        fun describeFailure(t: Throwable, url: String): String {
            val chain = generateSequence(t) { it.cause }.toList()
            if (chain.any { it is java.security.cert.CertificateException }) return PIN_MISMATCH
            val host = hostOf(url)
            val overCable = isLoopback(host)
            val text = chain.joinToString(" ") { it.message.orEmpty() }.lowercase()
            return when {
                chain.any { it is java.net.SocketTimeoutException } ->
                    if (overCable) NOTHING_ON_USB else "$host didn't answer. Is this tablet on the same network as the computer?"
                chain.any { it is java.net.NoRouteToHostException || it is java.net.UnknownHostException } ||
                    "unreachable" in text -> "Can't reach $host from this network."
                chain.any { it is java.net.ConnectException } ->
                    if (overCable) NOTHING_ON_USB else "Nothing answers at $host. Is PerritoDeck running on the computer?"
                // Over the cable, adb on the tablet takes the connection and then drops it
                // when nothing listens on the computer's end.
                chain.any { it is javax.net.ssl.SSLException || it is java.net.SocketException || it is java.io.EOFException } ->
                    if (overCable) NOT_RUNNING_ON_USB else "The connection to $host was cut. Is PerritoDeck running on the computer?"
                else -> (t.message ?: t.javaClass.simpleName).replace(Regex("ssl=0x[0-9a-fA-F]+: "), "")
            }
        }

        /**
         * Whether connecting failed the way a firewall makes it fail: the connection
         * attempt got no answer at all (Windows Firewall drops it silently). Not a refusal,
         * which is what an address with no PerritoDeck running gives (a restart, say), and
         * not when this tablet has no way there (Wi-Fi off, another network).
         */
        fun looksBlocked(t: Throwable): Boolean {
            val chain = generateSequence(t) { it.cause }.toList()
            val text = chain.joinToString(" ") { it.message.orEmpty() }.lowercase()
            if (chain.any { it is java.net.NoRouteToHostException || it is java.net.UnknownHostException } || "unreachable" in text) {
                return false
            }
            // "connect timed out", or Android's "failed to connect to /… after 10000ms"; a
            // read timeout means something did answer.
            return chain.any { it is java.net.SocketTimeoutException && "connect" in it.message.orEmpty().lowercase() }
        }

        private fun hostOf(url: String): String =
            url.substringAfter("://").substringBefore('/').substringBeforeLast(':').removeSurrounding("[", "]")

        const val PIN_MISMATCH =
            "This isn't the computer you paired with — its identity changed. If you reinstalled PerritoDeck Host, pair again."

        /** Slider moves are sent at most this often while dragging. */
        const val SLIDER_INTERVAL_MS = 50L

        /** On the cable because the network failed, the network is tried again this often. */
        const val NETWORK_RECHECK_MS = 30_000L

        /** 0.5s, 1s, 2s, 4s, then 8s between attempts. */
        fun backoffMs(attempt: Int): Long = (250L shl attempt.coerceIn(1, 5)).coerceAtMost(8_000)
    }
}
