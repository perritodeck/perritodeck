package dev.perritodeck.client.net

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
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
    private val _hosts = MutableStateFlow<List<FoundHost>>(emptyList())
    val hosts: StateFlow<List<FoundHost>> = _hosts.asStateFlow()

    private var listener: NsdManager.DiscoveryListener? = null
    private val queue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false

    fun start() {
        if (listener != null) return
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
                main.post { _hosts.value = _hosts.value.filterNot { it.name == info.serviceName } }
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
        resolving = true
        @Suppress("DEPRECATION") // resolveService is the only option before API 34
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                main.post { resolving = false; resolveNext() }
            }

            override fun onServiceResolved(info: NsdServiceInfo) {
                @Suppress("DEPRECATION")
                val address = info.host?.hostAddress
                val id = runCatching { info.attributes["id"]?.toString(Charsets.UTF_8) }.getOrNull()
                val name = runCatching { info.attributes["n"]?.toString(Charsets.UTF_8) }.getOrNull() ?: info.serviceName
                main.post {
                    if (address != null) {
                        val found = FoundHost(name, address, info.port, id)
                        _hosts.value = _hosts.value.filterNot { it.name == found.name || (id != null && it.hostId == id) } + found
                    }
                    resolving = false
                    resolveNext()
                }
            }
        })
    }

    companion object {
        const val SERVICE_TYPE = "_perritodeck._tcp."
    }
}
