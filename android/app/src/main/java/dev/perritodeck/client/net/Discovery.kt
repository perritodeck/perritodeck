package dev.perritodeck.client.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.ByteArrayOutputStream
import java.net.Inet4Address
import java.net.InetAddress
import java.util.ArrayDeque

/** A PerritoDeck host advertising itself over mDNS. */
data class FoundHost(val name: String, val address: String, val port: Int, val hostId: String?)

/**
 * Finds hosts advertising `_perritodeck._tcp` with the platform NsdManager (no extra
 * dependencies, works back to API 16). Resolves are queued because older Android
 * versions only allow one at a time.
 */
class Discovery(context: Context) {
    private val nsd = context.applicationContext.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val main = Handler(Looper.getMainLooper())
    private val found = FoundHosts()
    val hosts: StateFlow<List<FoundHost>> = found.hosts

    private var listener: NsdManager.DiscoveryListener? = null
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false

    fun start() {
        if (listener != null) return
        found.newRun()
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                main.post { listener = null }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            override fun onServiceFound(info: NsdServiceInfo) {
                main.post { queue.add(info); resolveNext() }
            }
            override fun onServiceLost(info: NsdServiceInfo) {
                main.post { found.lost(info.serviceName) }
            }
        }
        listener = l
        runCatching { nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l) }
            .onFailure { listener = null }
    }

    fun stop() {
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
        queue.clear()
    }

    private fun resolveNext() {
        if (resolving) return
        val next = queue.poll() ?: return
        // Keyed by the name onServiceFound gave, the one onServiceLost gives: before Android 7
        // the resolved info's serviceName is the daemon's DNS-escaped name instead.
        val service = next.serviceName
        resolving = true
        @Suppress("DEPRECATION") // resolveService is the only option before API 34
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                main.post { resolving = false; resolveNext() }
            }

            override fun onServiceResolved(info: NsdServiceInfo) {
                val address = usableAddress(addressesOf(info))
                val id = runCatching { info.attributes["id"]?.toString(Charsets.UTF_8) }.getOrNull()
                val name = runCatching { info.attributes["n"]?.toString(Charsets.UTF_8) }.getOrNull()
                    ?: unescapeServiceName(info.serviceName)
                main.post {
                    if (address != null) found.resolved(service, FoundHost(name, address, info.port, id))
                    resolving = false
                    resolveNext()
                }
            }
        })
    }

    @Suppress("DEPRECATION") // getHost, a single address, is all there is before API 34
    private fun addressesOf(info: NsdServiceInfo): List<InetAddress> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) info.hostAddresses
        else listOfNotNull(info.host)

    companion object {
        const val SERVICE_TYPE = "_perritodeck._tcp."
    }
}

/**
 * The hosts one discovery run has seen, for [Discovery] (on the main thread), by service name:
 * the host advertises "PerritoDeck on <computer>" and puts the computer's own name, the one
 * shown, in its "n" TXT record, and onServiceLost names the service. Names are compared
 * unescaped, as onServiceFound and onServiceLost report them, so a name taken from a resolved
 * NsdServiceInfo (escaped before Android 7: "PerritoDeck\032on\032...") still matches.
 */
internal class FoundHosts {
    private val byService = LinkedHashMap<String, FoundHost>()
    private val _hosts = MutableStateFlow<List<FoundHost>>(emptyList())
    val hosts: StateFlow<List<FoundHost>> = _hosts.asStateFlow()

    /**
     * A new run starts from nothing: a computer that went away while no run was listening (the
     * tablet's Wi-Fi went off while the deck was showing) is never reported lost.
     */
    fun newRun() {
        byService.clear()
        publish()
    }

    fun resolved(serviceName: String, host: FoundHost) {
        val key = unescapeServiceName(serviceName)
        // The same computer at a new address, or renamed (a new service name), replaces itself.
        byService.entries.removeAll { (service, seen) ->
            service == key || seen.name == host.name || (host.hostId != null && seen.hostId == host.hostId)
        }
        byService[key] = host
        publish()
    }

    fun lost(serviceName: String) {
        if (byService.remove(unescapeServiceName(serviceName)) != null) publish()
    }

    private fun publish() {
        _hosts.value = byService.values.toList()
    }
}

/**
 * A DNS-SD service name with mDNSResponder's escapes undone: `\.` and `\\` for a dot and a
 * backslash, `\ddd` (decimal) for any other byte, such as `\032` for a space. The bytes are
 * UTF-8. A backslash that starts no complete escape is kept as it is.
 */
internal fun unescapeServiceName(name: String): String {
    if ('\\' !in name) return name
    val out = ByteArrayOutputStream(name.length)
    val plain = StringBuilder()
    fun flush() {
        out.write(plain.toString().toByteArray(Charsets.UTF_8))
        plain.setLength(0)
    }
    var i = 0
    while (i < name.length) {
        val c = name[i]
        if (c != '\\' || i + 1 == name.length) {
            plain.append(c)
            i++
            continue
        }
        val digits = name.substring(i + 1, minOf(i + 4, name.length))
        val byte = digits.takeIf { d -> d.length == 3 && d.all { it in '0'..'9' } }?.toInt()
        when {
            byte != null && byte <= 255 -> { flush(); out.write(byte); i += 4 }
            name[i + 1] in '0'..'9' -> { plain.append(c); i++ }
            else -> { plain.append(name[i + 1]); i += 2 }
        }
    }
    flush()
    return out.toByteArray().toString(Charsets.UTF_8)
}

/**
 * The address to connect to out of those a host resolved to: IPv4 when there is one, as
 * the host listens on IPv4 only. An IPv6 link-local address needs an interface name,
 * which a URL can't carry, so it doesn't count.
 */
internal fun usableAddress(addresses: List<InetAddress>): String? =
    (addresses.firstOrNull { it is Inet4Address } ?: addresses.firstOrNull { !it.isLinkLocalAddress })
        ?.hostAddress?.substringBefore('%')
