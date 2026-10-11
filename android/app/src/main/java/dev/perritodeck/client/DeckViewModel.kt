package dev.perritodeck.client

import android.app.Application
import android.content.Context
import android.os.Build
import android.util.DisplayMetrics
import android.view.WindowManager
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.perritodeck.client.data.HostTarget
import dev.perritodeck.client.data.Settings
import dev.perritodeck.client.net.DeckConnection
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.Platform
import dev.perritodeck.client.protocol.Screen
import okhttp3.OkHttpClient
import java.util.concurrent.TimeUnit

class DeckViewModel(app: Application) : AndroidViewModel(app) {
    val settings = Settings(app)

    private val http = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS) // the protocol's own heartbeat detects dead hosts
        .build()

    val connection = DeckConnection(http, viewModelScope)

    fun connect(target: HostTarget) {
        settings.lastTarget = target
        connection.connect(target.url, buildHello(target.token))
    }

    fun disconnect() = connection.disconnect()

    private fun buildHello(token: String): Hello {
        val app = getApplication<Application>()
        return Hello(
            deviceId = settings.deviceId,
            deviceName = deviceName(),
            appVersion = BuildConfig.VERSION_NAME,
            platform = Platform.ANDROID,
            osVersion = Build.VERSION.RELEASE ?: "unknown",
            screen = screen(app),
            capabilities = listOf("keys", "haptics", "multiTouch"),
            token = token.trim().ifEmpty { null },
        )
    }

    private fun deviceName(): String {
        val maker = Build.MANUFACTURER.orEmpty().replaceFirstChar { it.uppercase() }
        val model = Build.MODEL.orEmpty()
        return if (model.startsWith(maker, ignoreCase = true)) model else "$maker $model".trim()
    }

    @Suppress("DEPRECATION") // getRealMetrics is the only API that works back to API 22
    private fun screen(context: Context): Screen {
        val dm = DisplayMetrics()
        val wm = context.getSystemService(Context.WINDOW_SERVICE) as WindowManager
        wm.defaultDisplay.getRealMetrics(dm)
        return Screen(widthPx = dm.widthPixels, heightPx = dm.heightPixels, densityDpi = dm.densityDpi)
    }

    override fun onCleared() {
        connection.disconnect()
        http.dispatcher.executorService.shutdown()
        http.connectionPool.evictAll()
    }
}
