package dev.perritodeck.client.data

import android.content.Context
import dev.perritodeck.client.protocol.DEFAULT_PORT
import java.util.UUID

data class HostTarget(val host: String, val port: Int, val token: String) {
    val url: String get() = "ws://${formatHost(host)}:$port"

    companion object {
        /** Bracket bare IPv6 literals so they form a valid URL. */
        fun formatHost(host: String): String =
            if (host.contains(':') && !host.startsWith("[")) "[$host]" else host
    }
}

/** Small key-value store for the connection form and the per-install device id. */
class Settings(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("perrito", Context.MODE_PRIVATE)

    /** Stable id the host uses to recognise this tablet across reconnects. */
    val deviceId: String
        get() = prefs.getString(KEY_DEVICE_ID, null) ?: UUID.randomUUID().toString().also {
            prefs.edit().putString(KEY_DEVICE_ID, it).apply()
        }

    var lastTarget: HostTarget
        get() = HostTarget(
            host = prefs.getString(KEY_HOST, null) ?: "10.0.2.2",
            port = prefs.getInt(KEY_PORT, DEFAULT_PORT),
            token = prefs.getString(KEY_TOKEN, null) ?: "",
        )
        set(value) {
            prefs.edit()
                .putString(KEY_HOST, value.host)
                .putInt(KEY_PORT, value.port)
                .putString(KEY_TOKEN, value.token)
                .apply()
        }

    private companion object {
        const val KEY_DEVICE_ID = "deviceId"
        const val KEY_HOST = "host"
        const val KEY_PORT = "port"
        const val KEY_TOKEN = "token"
    }
}
