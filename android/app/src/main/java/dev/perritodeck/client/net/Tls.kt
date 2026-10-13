package dev.perritodeck.client.net

import okhttp3.OkHttpClient
import java.net.InetSocketAddress
import java.net.Socket
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/**
 * The host uses a self-signed certificate; instead of a CA chain we pin the SHA-256 of
 * its certificate (base64url, no padding), learned when pairing. Same format as the
 * host's `HostIdentity::fingerprint`.
 */
object Fingerprint {
    fun of(certDer: ByteArray): String = Base64Url.encode(MessageDigest.getInstance("SHA-256").digest(certDer))

    /** Short form for comparing by eye, e.g. `3FA9 07C2 D41E` (first 6 bytes). Matches the host. */
    fun display(fingerprint: String): String {
        val bytes = Base64Url.decode(fingerprint) ?: return "?"
        return bytes.take(6).joinToString("") { "%02X".format(it.toInt() and 0xFF) }.chunked(4).joinToString(" ")
    }
}

/** base64url without padding; java.util.Base64 needs API 26 and android.util.Base64 isn't on the JVM. */
object Base64Url {
    private const val ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"

    fun encode(data: ByteArray): String {
        val out = StringBuilder((data.size * 4 + 2) / 3)
        var i = 0
        while (i < data.size) {
            val b0 = data[i].toInt() and 0xFF
            val b1 = if (i + 1 < data.size) data[i + 1].toInt() and 0xFF else -1
            val b2 = if (i + 2 < data.size) data[i + 2].toInt() and 0xFF else -1
            out.append(ALPHABET[b0 shr 2])
            out.append(ALPHABET[((b0 and 0x3) shl 4) or (if (b1 >= 0) b1 shr 4 else 0)])
            if (b1 >= 0) out.append(ALPHABET[((b1 and 0xF) shl 2) or (if (b2 >= 0) b2 shr 6 else 0)])
            if (b2 >= 0) out.append(ALPHABET[b2 and 0x3F])
            i += 3
        }
        return out.toString()
    }

    fun decode(s: String): ByteArray? {
        val out = java.io.ByteArrayOutputStream(s.length * 3 / 4)
        var buffer = 0
        var bits = 0
        for (c in s) {
            val v = ALPHABET.indexOf(c)
            if (v < 0) return null
            buffer = (buffer shl 6) or v
            bits += 6
            if (bits >= 8) {
                bits -= 8
                out.write((buffer shr bits) and 0xFF)
            }
        }
        return out.toByteArray()
    }
}

/** Accepts exactly one certificate (by fingerprint), or — when [expected] is null — any, recording what it saw. */
class PinningTrustManager(private val expected: String?) : X509TrustManager {
    @Volatile var seen: String? = null
        private set

    override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
        val leaf = chain?.firstOrNull() ?: throw CertificateException("host sent no certificate")
        val fp = Fingerprint.of(leaf.encoded)
        seen = fp
        if (expected != null && fp != expected) {
            throw CertificateException("This isn't the computer you paired with (fingerprint changed). Pair again if you reinstalled the host.")
        }
    }

    override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) =
        throw CertificateException("client certificates are not used")

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}

object Tls {
    private fun context(tm: X509TrustManager): SSLContext =
        SSLContext.getInstance("TLS").apply { init(null, arrayOf(tm), null) }

    /** An OkHttp client that only talks to the host with this fingerprint. */
    fun pinnedClient(base: OkHttpClient, fingerprint: String): OkHttpClient {
        val tm = PinningTrustManager(fingerprint)
        return base.newBuilder()
            .sslSocketFactory(context(tm).socketFactory, tm)
            // The certificate is pinned, so its name doesn't matter (hosts are reached by IP).
            .hostnameVerifier { _, _ -> true }
            .build()
    }

    /**
     * Connect once and return the fingerprint the host presents (blocking; call off the
     * main thread). Closing [raw] from another thread ends it early.
     */
    fun probe(host: String, port: Int, timeoutMs: Int = 5_000, raw: Socket = Socket()): String {
        val tm = PinningTrustManager(null)
        raw.use {
            raw.connect(InetSocketAddress(host, port), timeoutMs)
            raw.soTimeout = timeoutMs
            (context(tm).socketFactory.createSocket(raw, host, port, true) as SSLSocket).use { tls ->
                tls.startHandshake()
            }
        }
        return tm.seen ?: throw CertificateException("no certificate received")
    }

    val defaultClient: OkHttpClient by lazy {
        OkHttpClient.Builder()
            .connectTimeout(5, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.MILLISECONDS) // the protocol's heartbeat detects dead hosts
            .build()
    }
}
