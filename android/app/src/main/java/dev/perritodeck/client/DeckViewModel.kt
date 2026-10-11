package dev.perritodeck.client

import android.app.Application
import android.content.Context
import android.os.Build
import android.util.DisplayMetrics
import android.view.WindowManager
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.perritodeck.client.data.HostPort
import dev.perritodeck.client.data.PairedHost
import dev.perritodeck.client.data.PairingLink
import dev.perritodeck.client.data.Settings
import dev.perritodeck.client.net.ConnState
import dev.perritodeck.client.net.DeckConnection
import dev.perritodeck.client.net.Discovery
import dev.perritodeck.client.net.Pairing
import dev.perritodeck.client.net.PairingException
import dev.perritodeck.client.net.Tls
import dev.perritodeck.client.protocol.DEFAULT_PORT
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.Platform
import dev.perritodeck.client.protocol.Screen
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Which top-level screen is showing. */
sealed interface Route {
    data object Home : Route
    data object Pair : Route
    data object Deck : Route
}

/** Where the pairing flow is. */
sealed interface PairState {
    data class Form(val address: String, val error: String? = null) : PairState
    data class Working(val message: String) : PairState
    /** Manual pairing: the user compares this fingerprint with the one the computer shows. */
    data class Confirm(val host: String, val port: Int, val code: String, val fingerprint: String) : PairState
}

class DeckViewModel(app: Application) : AndroidViewModel(app) {
    val settings = Settings(app)
    val discovery = Discovery(app)
    val connection = DeckConnection(Tls.defaultClient, viewModelScope)

    private val _route = MutableStateFlow<Route>(Route.Home)
    val route: StateFlow<Route> = _route.asStateFlow()

    private val _paired = MutableStateFlow(settings.pairedHosts)
    val paired: StateFlow<List<PairedHost>> = _paired.asStateFlow()

    private val _pairState = MutableStateFlow<PairState>(PairState.Form(settings.lastManualAddress))
    val pairState: StateFlow<PairState> = _pairState.asStateFlow()

    /** Shown on the home screen (e.g. why the last connection ended). */
    private val _notice = MutableStateFlow<String?>(null)
    val notice: StateFlow<String?> = _notice.asStateFlow()

    private var pairJob: Job? = null

    init {
        // A dedicated deck tablet should come straight back to its computer after a restart.
        settings.lastHostId?.let { id -> _paired.value.firstOrNull { it.hostId == id } }?.let(::connectTo)

        viewModelScope.launch {
            connection.state.collect { s ->
                if (s is ConnState.Failed && _route.value == Route.Deck) {
                    _notice.value = s.reason
                    _route.value = Route.Home
                }
            }
        }
    }

    fun connectTo(host: PairedHost) {
        // Prefer the address discovery just saw (DHCP may have moved the computer).
        val seen = discovery.hosts.value.firstOrNull { it.hostId == host.hostId }
        val target = if (seen != null && (seen.address != host.address || seen.port != host.port)) {
            settings.updateAddress(host.hostId, seen.address, seen.port)
            host.copy(address = seen.address, port = seen.port)
        } else {
            host
        }
        _paired.value = settings.pairedHosts
        settings.lastHostId = host.hostId
        _notice.value = null
        connection.connect(target.url, hello(target.deviceSecret), Tls.pinnedClient(Tls.defaultClient, target.hostId))
        _route.value = Route.Deck
    }

    fun disconnect() {
        connection.disconnect()
        settings.lastHostId = null
        _route.value = Route.Home
    }

    fun forget(host: PairedHost) {
        settings.forget(host.hostId)
        _paired.value = settings.pairedHosts
    }

    fun openPairing(prefillAddress: String? = null) {
        pairJob?.cancel()
        _pairState.value = PairState.Form(prefillAddress ?: settings.lastManualAddress)
        _route.value = Route.Pair
    }

    fun closePairing() {
        pairJob?.cancel()
        _route.value = Route.Home
    }

    /** Manual pairing step 1: reach the computer and show its fingerprint for comparison. */
    fun submitCode(address: String, code: String) {
        val hp = parseAddress(address)
        val digits = code.filter(Char::isDigit)
        when {
            hp == null -> _pairState.value = PairState.Form(address, "Enter the computer's address, like 192.168.1.20")
            digits.length != 6 -> _pairState.value = PairState.Form(address, "The code has 6 digits")
            else -> {
                settings.lastManualAddress = address.trim()
                pairJob?.cancel()
                pairJob = viewModelScope.launch {
                    _pairState.value = PairState.Working("Reaching ${hp.host}…")
                    _pairState.value = try {
                        PairState.Confirm(hp.host, hp.port, digits, Pairing.probe(hp.host, hp.port))
                    } catch (e: PairingException) {
                        PairState.Form(address, e.message)
                    }
                }
            }
        }
    }

    /** Manual pairing step 2: the fingerprints match, so send the code. */
    fun confirmPairing() {
        val c = _pairState.value as? PairState.Confirm ?: return
        pairJob?.cancel()
        pairJob = viewModelScope.launch { finishPairing(c.host, c.port, c.fingerprint, c.code, "${c.host}:${c.port}") }
    }

    /** A scanned QR link carries the fingerprint, so no comparison step is needed. */
    fun handleLink(uri: String) {
        val link = PairingLink.parse(uri) ?: run {
            _notice.value = "That pairing link isn't valid. Show a new code on the computer and scan again."
            return
        }
        connection.disconnect()
        _route.value = Route.Pair
        pairJob?.cancel()
        pairJob = viewModelScope.launch {
            _pairState.value = PairState.Working("Pairing with ${link.hostName}…")
            // The host lists every address it has; use the first one this tablet can reach.
            for (addr in link.addresses) {
                val fp = runCatching { Pairing.probe(addr.host, addr.port) }.getOrNull() ?: continue
                if (fp != link.fingerprint) continue
                finishPairing(addr.host, addr.port, link.fingerprint, link.code, "${addr.host}:${addr.port}")
                return@launch
            }
            _pairState.value = PairState.Form(
                link.addresses.first().let { "${it.host}:${it.port}" },
                "Couldn't reach ${link.hostName}. Make sure this tablet is on the same Wi-Fi network.",
            )
        }
    }

    private suspend fun finishPairing(host: String, port: Int, fingerprint: String, code: String, address: String) {
        _pairState.value = PairState.Working("Pairing…")
        try {
            val paired = Pairing.pair(Tls.defaultClient, host, port, fingerprint, code, settings.deviceId, deviceName())
            settings.savePaired(paired)
            _paired.value = settings.pairedHosts
            _pairState.value = PairState.Form("")
            connectTo(paired)
        } catch (e: PairingException) {
            _pairState.value = PairState.Form(address, e.message)
        } catch (e: kotlinx.coroutines.TimeoutCancellationException) {
            _pairState.value = PairState.Form(address, "The computer didn't answer. Try again.")
        }
    }

    private fun parseAddress(s: String): HostPort? {
        val t = s.trim()
        if (t.isEmpty()) return null
        return HostPort.parse(t) ?: HostPort.parse("$t:$DEFAULT_PORT")
    }

    private fun hello(secret: String): Hello {
        val app = getApplication<Application>()
        return Hello(
            deviceId = settings.deviceId,
            deviceName = deviceName(),
            appVersion = BuildConfig.VERSION_NAME,
            platform = Platform.ANDROID,
            osVersion = Build.VERSION.RELEASE ?: "unknown",
            screen = screen(app),
            capabilities = listOf("keys", "haptics", "multiTouch", "images"),
            deviceSecret = secret,
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
        discovery.stop()
    }
}
