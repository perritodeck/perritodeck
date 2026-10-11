package dev.perritodeck.client.net

import dev.perritodeck.client.protocol.Codec
import dev.perritodeck.client.protocol.Decoded
import dev.perritodeck.client.protocol.Envelope
import dev.perritodeck.client.protocol.ErrorCode
import dev.perritodeck.client.protocol.Heartbeat
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.KeyEvent
import dev.perritodeck.client.protocol.Layout
import dev.perritodeck.client.protocol.Message
import dev.perritodeck.client.protocol.ProtocolException
import dev.perritodeck.client.protocol.UnsupportedVersionException
import dev.perritodeck.client.protocol.Welcome
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

sealed interface ConnState {
    data object Idle : ConnState
    data class Connecting(val url: String, val attempt: Int, val lastError: String?) : ConnState
    data class Connected(val welcome: Welcome) : ConnState
    /** The host refused us for a reason retrying won't fix (bad token, wrong version). */
    data class Failed(val reason: String) : ConnState
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
) {
    private val _state = MutableStateFlow<ConnState>(ConnState.Idle)
    val state: StateFlow<ConnState> = _state.asStateFlow()

    private val _layout = MutableStateFlow<Layout?>(null)
    /** Last layout received; kept while reconnecting so the grid stays on screen. */
    val layout: StateFlow<Layout?> = _layout.asStateFlow()

    private val _rttMs = MutableStateFlow<Double?>(null)
    /** Round trip of the most recent key press, in milliseconds. */
    val rttMs: StateFlow<Double?> = _rttMs.asStateFlow()

    private var job: Job? = null
    @Volatile private var socket: WebSocket? = null
    private val nextId = AtomicLong(1)
    private val pendingAcks = ConcurrentHashMap<Long, Long>()

    fun connect(url: String, hello: Hello) {
        disconnect()
        _layout.value = null
        job = scope.launch { runLoop(url, hello) }
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

    private fun sendKey(msg: Message) {
        val ws = socket ?: return
        if (_state.value !is ConnState.Connected) return
        val id = nextId.getAndIncrement()
        if (pendingAcks.size > 256) pendingAcks.clear()
        pendingAcks[id] = nanoTime()
        ws.send(Codec.encode(Envelope(msg, id)))
    }

    private suspend fun runLoop(url: String, hello: Hello) {
        var attempt = 0
        var lastError: String? = null
        while (currentCoroutineContext().isActive) {
            attempt++
            _state.value = ConnState.Connecting(url, attempt, lastError)
            when (val end = runSession(url, hello)) {
                is SessionEnd.Fatal -> {
                    _state.value = ConnState.Failed(end.reason)
                    return
                }
                is SessionEnd.Retry -> {
                    lastError = end.reason
                    if (end.wasConnected) attempt = 0
                }
            }
            delay(backoffMs(attempt))
        }
    }

    private sealed interface SessionEnd {
        data class Retry(val reason: String, val wasConnected: Boolean) : SessionEnd
        data class Fatal(val reason: String) : SessionEnd
    }

    private sealed interface WsEvent {
        data object Open : WsEvent
        data class Text(val text: String) : WsEvent
        data class Closed(val reason: String) : WsEvent
        data class Failure(val reason: String) : WsEvent
    }

    private suspend fun runSession(url: String, hello: Hello): SessionEnd {
        val events = Channel<WsEvent>(Channel.UNLIMITED)
        val request = try {
            Request.Builder().url(url).build()
        } catch (e: IllegalArgumentException) {
            return SessionEnd.Fatal("Invalid address: $url")
        }
        val ws = http.newWebSocket(request, object : WebSocketListener() {
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
                events.trySend(WsEvent.Failure(t.message ?: t.javaClass.simpleName))
            }
        })
        socket = ws
        var connected = false
        var heartbeat: Job? = null
        val lastHeard = AtomicLong(nanoTime())
        try {
            when (val first = events.receive()) {
                WsEvent.Open -> ws.send(Codec.encode(Envelope(Message.HelloMsg(hello), nextId.getAndIncrement())))
                is WsEvent.Failure -> return SessionEnd.Retry(first.reason, false)
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
                                _state.value = ConnState.Connected(m.payload)
                                heartbeat?.cancel()
                                heartbeat = startHeartbeat(ws, m.payload.heartbeatIntervalMs, lastHeard)
                            }
                            is Message.LayoutMsg -> _layout.value = m.payload
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
                                ErrorCode.UNSUPPORTED_VERSION -> return SessionEnd.Fatal(m.payload.message)
                                else -> Unit // non-fatal; Phase 2 surfaces these in a debug panel
                            }
                            else -> Unit // setImage arrives in Phase 2; heartbeatAck only refreshes lastHeard
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
        /** 0.5s, 1s, 2s, 4s, then 8s between attempts. */
        fun backoffMs(attempt: Int): Long = (250L shl attempt.coerceIn(1, 5)).coerceAtMost(8_000)
    }
}
