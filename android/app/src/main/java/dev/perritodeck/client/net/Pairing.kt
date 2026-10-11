package dev.perritodeck.client.net

import dev.perritodeck.client.data.PairedHost
import dev.perritodeck.client.protocol.Codec
import dev.perritodeck.client.protocol.Decoded
import dev.perritodeck.client.protocol.Envelope
import dev.perritodeck.client.protocol.Message
import dev.perritodeck.client.protocol.PairRequest
import dev.perritodeck.client.data.formatHost
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import kotlin.coroutines.resume

class PairingException(message: String) : Exception(message)

object Pairing {
    /** Reach the host and learn its certificate fingerprint (for the user to compare). */
    suspend fun probe(host: String, port: Int): String = withContext(Dispatchers.IO) {
        try {
            Tls.probe(host, port)
        } catch (e: Exception) {
            throw PairingException("Couldn't reach $host:$port — is PerritoDeck Host running, and on the same network? (${e.message})")
        }
    }

    /**
     * Send the one-time code over a connection pinned to [fingerprint]. On success the
     * host closes the connection and we reconnect normally with the issued secret.
     */
    suspend fun pair(
        base: OkHttpClient,
        host: String,
        port: Int,
        fingerprint: String,
        code: String,
        deviceId: String,
        deviceName: String,
    ): PairedHost {
        val client = Tls.pinnedClient(base, fingerprint)
        val request = Request.Builder().url("wss://${formatHost(host)}:$port").build()
        val result: Result<Message> = withTimeout(10_000) {
            suspendCancellableCoroutine { cont ->
                val ws = client.newWebSocket(request, object : WebSocketListener() {
                    override fun onOpen(webSocket: WebSocket, response: Response) {
                        webSocket.send(Codec.encode(Envelope(Message.PairMsg(PairRequest(code, deviceId, deviceName)), 1)))
                    }

                    override fun onMessage(webSocket: WebSocket, text: String) {
                        val msg = runCatching { (Codec.decode(text) as? Decoded.Known)?.envelope?.msg }.getOrNull() ?: return
                        if (cont.isActive) cont.resume(Result.success(msg))
                        webSocket.close(1000, null)
                    }

                    override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                        if (cont.isActive) cont.resume(Result.failure(PairingException(t.message ?: "connection failed")))
                    }

                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                        if (cont.isActive) cont.resume(Result.failure(PairingException("The computer closed the connection.")))
                    }
                })
                cont.invokeOnCancellation { ws.cancel() }
            }
        }
        return when (val m = result.getOrThrow()) {
            is Message.PairedMsg -> {
                if (m.payload.hostId != fingerprint) throw PairingException("The computer's identity changed during pairing. Try again.")
                PairedHost(
                    hostId = fingerprint,
                    name = m.payload.hostName,
                    address = host,
                    port = port,
                    deviceSecret = m.payload.deviceSecret,
                )
            }
            is Message.Error -> throw PairingException(m.payload.message.replaceFirstChar { it.uppercase() })
            else -> throw PairingException("Unexpected reply from the computer (${m.type}).")
        }
    }
}
