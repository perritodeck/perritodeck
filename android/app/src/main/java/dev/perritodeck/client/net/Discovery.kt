package dev.perritodeck.client.net

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
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
 * dependencies, works back to API 16), while Home wants it ([start], [stop]) and a Wi-Fi or
 * Ethernet network is up. NsdManager can't be trusted to notice the network changing: on
 * the Fire 7 (Fire OS 7) it reported no lost service while Wi-Fi was off, and after Wi-Fi
 * came back its mdnsd found nothing until the app's process ended. So losing a network
 * clears the list at once, and a new network starts a new run; [onNetworkChange] hears the
 * same changes (true: a network came up), on the main thread. Resolves are queued because
 * older Android versions only allow one at a time, and each gives up after
 * [RESOLVE_TIMEOUT_MS]. While a run is on, a Wi-Fi multicast lock keeps the hosts'
 * announcements and goodbyes coming: without one, Wi-Fi may drop multicast packets.
 */
class Discovery(context: Context, private val onNetworkChange: (up: Boolean) -> Unit = {}) {
    private val app = context.applicationContext
    private val nsd = app.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val connectivity = app.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
    private val multicast = (context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager?)
        ?.createMulticastLock("perrito-discovery")
        ?.apply { setReferenceCounted(false) }
    private val main = Handler(Looper.getMainLooper())
    private val found = FoundHosts()
    val hosts: StateFlow<List<FoundHost>> = found.hosts

    private var listener: NsdManager.DiscoveryListener? = null
    private var resolving: NsdManager.ResolveListener? = null
    private val resolves = ResolveQueue<NsdServiceInfo>(
        RESOLVE_TIMEOUT_MS,
        after = { delayMs, action ->
            val r = Runnable(action)
            main.postDelayed(r, delayMs)
            ({ main.removeCallbacks(r) })
        },
        start = ::resolve,
        abandon = { _, _ -> abandonResolve() },
    )
    private val runs = DiscoveryRuns<Network>(found, begin = ::begin, end = ::end)

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            main.post { if (runs.available(network)) onNetworkChange(true) }
        }

        override fun onLost(network: Network) {
            main.post { if (runs.lost(network)) onNetworkChange(false) }
        }
    }

    init {
        runs.seed(lanNetworks())
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
            // A deck's network needn't reach the internet.
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        runCatching { connectivity.registerNetworkCallback(request, networkCallback) }
    }

    fun start() = runs.start()

    fun stop() = runs.stop()

    /** Stops for good (the ViewModel is cleared). */
    fun close() {
        stop()
        runCatching { connectivity.unregisterNetworkCallback(networkCallback) }
    }

    /** The Wi-Fi and Ethernet networks up now; the callback reports them again once it's registered. */
    @Suppress("DEPRECATION") // allNetworks is the only snapshot there is on API 22
    private fun lanNetworks(): List<Network> = runCatching {
        connectivity.allNetworks.filter { n ->
            val caps = connectivity.getNetworkCapabilities(n)
            caps != null && LAN_TRANSPORTS.any(caps::hasTransport)
        }
    }.getOrDefault(emptyList())

    private fun begin() {
        multicast?.let { lock -> if (!lock.isHeld) runCatching { lock.acquire() } }
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                main.post { if (listener === this) failed() }
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            // A run that has ended may still hear from NsdManager; only the current one counts.
            override fun onServiceFound(info: NsdServiceInfo) {
                main.post { if (listener === this) resolves.add(info) }
            }
            override fun onServiceLost(info: NsdServiceInfo) {
                main.post { if (listener === this) found.lost(info.serviceName) }
            }
        }
        listener = l
        runCatching { nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, l) }
            .onFailure { failed() }
    }

    private fun failed() {
        listener = null
        releaseMulticast()
        runs.beginFailed()
    }

    private fun end() {
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
        resolves.clear()
        releaseMulticast()
    }

    private fun releaseMulticast() {
        multicast?.let { lock -> if (lock.isHeld) runCatching { lock.release() } }
    }

    private fun resolve(id: Long, next: NsdServiceInfo) {
        // Keyed by the name onServiceFound gave, the one onServiceLost gives: before Android 7
        // the resolved info's serviceName is the daemon's DNS-escaped name instead.
        val service = next.serviceName
        val l = object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                main.post { resolves.finished(id) }
            }

            override fun onServiceResolved(info: NsdServiceInfo) {
                val address = usableAddress(addressesOf(info))
                val hostId = runCatching { info.attributes["id"]?.toString(Charsets.UTF_8) }.getOrNull()
                val name = runCatching { info.attributes["n"]?.toString(Charsets.UTF_8) }.getOrNull()
                    ?: unescapeServiceName(info.serviceName)
                main.post {
                    // An answer after the timeout, or for a run that has ended, is dropped.
                    if (resolves.finished(id) && address != null) {
                        found.resolved(service, FoundHost(name, address, info.port, hostId))
                    }
                }
            }
        }
        resolving = l
        @Suppress("DEPRECATION") // resolveService is the only option before API 34
        runCatching { nsd.resolveService(next, l) }.onFailure { resolves.finished(id) }
    }

    /**
     * A resolve that timed out, or whose run ended. Android 14 can cancel it; before that
     * NsdService gives each app one resolve at a time and frees it only when mdnsd answers,
     * so later resolves may fail at once (FAILURE_ALREADY_ACTIVE) until it does.
     */
    private fun abandonResolve() {
        val l = resolving ?: return
        resolving = null
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            runCatching { nsd.stopServiceResolution(l) }
        }
    }

    @Suppress("DEPRECATION") // getHost, a single address, is all there is before API 34
    private fun addressesOf(info: NsdServiceInfo): List<InetAddress> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) info.hostAddresses
        else listOfNotNull(info.host)

    companion object {
        const val SERVICE_TYPE = "_perritodeck._tcp."

        /** A resolve normally answers within a second or two; on the Fire 7 a stale one never did. */
        const val RESOLVE_TIMEOUT_MS = 8_000L

        private val LAN_TRANSPORTS = intArrayOf(NetworkCapabilities.TRANSPORT_WIFI, NetworkCapabilities.TRANSPORT_ETHERNET)
    }
}

/**
 * When [Discovery] runs (on the main thread): while Home wants it ([start], [stop]) and one
 * of the Wi-Fi or Ethernet networks [N] is up. Losing a network forgets what was found at
 * once, Home showing or not, and a network that's new (another one, or the same Wi-Fi
 * back) starts a new run, as a run that began on another network may never hear anything.
 * [begin] and [end] start and stop NsdManager's discovery; a run starts with nothing found.
 */
internal class DiscoveryRuns<N>(
    private val found: FoundHosts,
    private val begin: () -> Unit,
    private val end: () -> Unit,
) {
    private val networks = LinkedHashSet<N>()
    private var wanted = false
    var running = false
        private set

    /** The networks up when watching started; reporting one of them again is no change. */
    fun seed(current: Collection<N>) {
        networks.addAll(current)
        update()
    }

    fun start() {
        wanted = true
        update()
    }

    fun stop() {
        wanted = false
        update()
    }

    /** A network is up. True when it's new: discovery starts over. */
    fun available(network: N): Boolean {
        if (!networks.add(network)) return false
        restart()
        return true
    }

    /** A network went away. True when it was one of them: what was found is forgotten. */
    fun lost(network: N): Boolean {
        if (!networks.remove(network)) return false
        found.clear()
        restart()
        return true
    }

    /** NsdManager wouldn't start: the next [start] or new network tries again. */
    fun beginFailed() {
        running = false
    }

    private fun restart() {
        if (running) {
            running = false
            end()
        }
        update()
    }

    private fun update() {
        val should = wanted && networks.isNotEmpty()
        if (should && !running) {
            found.newRun()
            running = true
            begin()
        } else if (!should && running) {
            running = false
            end()
        }
    }
}

/**
 * Resolves [T]s one at a time, as older Android allows one per app (on the main thread). Each
 * [start]ed resolve ends with [finished] or, after [timeoutMs] ([after] schedules it), is
 * given up ([abandon]) so the next can go: on the Fire 7 a resolve of a service found while
 * Wi-Fi was off never called back, and the queue waited on it until the process died.
 */
internal class ResolveQueue<T>(
    private val timeoutMs: Long,
    /** Runs the action after the delay, on the same thread; returns what cancels it. */
    private val after: (delayMs: Long, action: () -> Unit) -> () -> Unit,
    private val start: (id: Long, item: T) -> Unit,
    private val abandon: (id: Long, item: T) -> Unit = { _, _ -> },
) {
    private class InFlight<T>(val id: Long, val item: T, val cancelTimeout: () -> Unit)

    private val waiting = ArrayDeque<T>()
    private var current: InFlight<T>? = null
    private var nextId = 0L

    val busy: Boolean get() = current != null

    fun add(item: T) {
        waiting.add(item)
        next()
    }

    /**
     * Resolve [id] answered or failed, and the next one starts. False when it's no longer
     * the one waited on (it timed out, or [clear] came first): its answer is to be ignored.
     */
    fun finished(id: Long): Boolean {
        val c = current?.takeIf { it.id == id } ?: return false
        c.cancelTimeout()
        current = null
        next()
        return true
    }

    /** Drops the queue and gives up on the resolve in flight (discovery stopped). */
    fun clear() {
        waiting.clear()
        val c = current ?: return
        current = null
        c.cancelTimeout()
        abandon(c.id, c.item)
    }

    private fun next() {
        if (current != null) return
        val item = waiting.poll() ?: return
        val id = nextId++
        val cancel = after(timeoutMs) {
            if (current?.id == id) {
                current = null
                abandon(id, item)
                next()
            }
        }
        current = InFlight(id, item, cancel)
        start(id, item)
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
    fun newRun() = clear()

    /** Forgets every host at once: the network they were found on went away. */
    fun clear() {
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
