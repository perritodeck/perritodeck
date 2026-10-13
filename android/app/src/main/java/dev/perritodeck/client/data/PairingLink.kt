package dev.perritodeck.client.data

import java.net.URLDecoder

/**
 * The link the host shows as a QR code:
 * `perritodeck://pair?v=1&n=<name>&a=<ip:port>[&a=…]&fp=<fingerprint>&c=<code>`.
 * Any camera app that scans it opens Perrito with everything needed to pair.
 */
data class PairingLink(
    val hostName: String,
    val addresses: List<HostPort>,
    val fingerprint: String,
    val code: String,
) {
    /**
     * Where to try pairing. 127.0.0.1 is tried too, whatever the link says: on a USB
     * cable the computer forwards its port there (adb reverse), which works even when
     * the network doesn't let the tablet through, and with no cable it's refused at once.
     * The fingerprint check keeps anything else answering there from passing as the computer.
     */
    val candidates: List<HostPort>
        get() = (listOf(HostPort(LOCALHOST, addresses.first().port)) + addresses).distinct()

    /**
     * The address to save for reconnecting after pairing through [used]. Over the cable
     * that's the link's first network address: every connect tries 127.0.0.1 by itself,
     * and unplugged the tablet should still find the computer over Wi-Fi.
     */
    fun addressToSave(used: HostPort): HostPort =
        if (isLoopback(used.host)) addresses.firstOrNull { !isLoopback(it.host) } ?: used else used

    companion object {
        fun parse(uri: String): PairingLink? {
            if (!uri.startsWith("perritodeck://pair?")) return null
            val params = uri.substringAfter('?').split('&').mapNotNull { part ->
                val i = part.indexOf('=')
                if (i <= 0) null else part.substring(0, i) to decode(part.substring(i + 1))
            }
            fun one(k: String) = params.firstOrNull { it.first == k }?.second
            if (one("v") != "1") return null
            val fp = one("fp")?.takeIf { it.length == 43 && it.all { c -> c.isLetterOrDigit() || c == '-' || c == '_' } }
                ?: return null
            val code = one("c")?.takeIf { it.length == 6 && it.all(Char::isDigit) } ?: return null
            val addrs = params.filter { it.first == "a" }.mapNotNull { HostPort.parse(it.second) }
            if (addrs.isEmpty()) return null
            return PairingLink(one("n") ?: "your computer", addrs, fp, code)
        }

        private fun decode(s: String): String = runCatching { URLDecoder.decode(s, "UTF-8") }.getOrDefault(s)
    }
}

data class HostPort(val host: String, val port: Int) {
    companion object {
        /** `1.2.3.4:46710` or `[fe80::1]:46710`. */
        fun parse(s: String): HostPort? {
            val (host, port) = if (s.startsWith("[")) {
                val end = s.indexOf(']').takeIf { it > 0 } ?: return null
                s.substring(1, end) to s.substring(end + 1).removePrefix(":")
            } else {
                val i = s.lastIndexOf(':').takeIf { it > 0 } ?: return null
                s.substring(0, i) to s.substring(i + 1)
            }
            val p = port.toIntOrNull()?.takeIf { it in 1..65535 } ?: return null
            return if (host.isBlank()) null else HostPort(host, p)
        }
    }
}

const val LOCALHOST = "127.0.0.1"

/** This tablet itself, which over a USB cable is the computer too (adb reverse). */
fun isLoopback(host: String): Boolean = host == LOCALHOST || host == "localhost" || host == "::1"
