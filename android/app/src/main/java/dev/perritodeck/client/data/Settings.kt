package dev.perritodeck.client.data

import android.content.Context
import dev.perritodeck.client.protocol.Codec
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import java.util.UUID

/** A computer this tablet has paired with. */
@Serializable
data class PairedHost(
    /** Certificate fingerprint; the pin for every connection. */
    val hostId: String,
    val name: String,
    /** Last address that worked (discovery may supply a newer one). */
    val address: String,
    val port: Int,
    val deviceSecret: String,
) {
    val url: String get() = "wss://${formatHost(address)}:$port"

    /**
     * Where to reach this computer, best first: 127.0.0.1, which a USB cable forwards to
     * the computer (adb reverse) and which is refused at once without one, then the
     * address discovery just [seen] it at, then the saved one.
     */
    fun urls(seen: HostPort? = null): List<String> =
        listOfNotNull(HostPort(LOCALHOST, seen?.port ?: port).url, seen?.url, url).distinct()
}

/** Bracket bare IPv6 literals so they form a valid URL. */
fun formatHost(host: String): String = if (host.contains(':') && !host.startsWith("[")) "[$host]" else host

/** App-private storage for the device id and paired hosts. */
class Settings(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("perrito", Context.MODE_PRIVATE)
    private val hostsSerializer = ListSerializer(PairedHost.serializer())

    /** Stable id the host uses to recognise this tablet across reconnects. */
    val deviceId: String
        get() = prefs.getString(KEY_DEVICE_ID, null) ?: UUID.randomUUID().toString().also {
            prefs.edit().putString(KEY_DEVICE_ID, it).apply()
        }

    var pairedHosts: List<PairedHost>
        get() = prefs.getString(KEY_HOSTS, null)?.let {
            runCatching { Codec.json.decodeFromString(hostsSerializer, it) }.getOrNull()
        } ?: emptyList()
        private set(value) {
            prefs.edit().putString(KEY_HOSTS, Codec.json.encodeToString(hostsSerializer, value)).apply()
        }

    /** The host to reconnect to automatically when the app starts. */
    var lastHostId: String?
        get() = prefs.getString(KEY_LAST_HOST, null)
        set(value) {
            prefs.edit().putString(KEY_LAST_HOST, value).apply()
        }

    /** Address and port typed last time on the pairing screen. */
    var lastManualAddress: String
        get() = prefs.getString(KEY_MANUAL, null) ?: ""
        set(value) {
            prefs.edit().putString(KEY_MANUAL, value).apply()
        }

    fun savePaired(host: PairedHost) {
        pairedHosts = pairedHosts.filterNot { it.hostId == host.hostId } + host
        lastHostId = host.hostId
    }

    fun updateAddress(hostId: String, address: String, port: Int) {
        pairedHosts = pairedHosts.map { if (it.hostId == hostId) it.copy(address = address, port = port) else it }
    }

    fun forget(hostId: String) {
        pairedHosts = pairedHosts.filterNot { it.hostId == hostId }
        if (lastHostId == hostId) lastHostId = null
    }

    private companion object {
        const val KEY_DEVICE_ID = "deviceId"
        const val KEY_HOSTS = "pairedHosts"
        const val KEY_LAST_HOST = "lastHostId"
        const val KEY_MANUAL = "lastManualAddress"
    }
}
