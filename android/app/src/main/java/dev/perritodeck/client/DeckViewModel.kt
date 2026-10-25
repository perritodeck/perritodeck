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
import dev.perritodeck.client.net.SavedAddressCheck
import dev.perritodeck.client.net.Tls
import dev.perritodeck.client.protocol.DEFAULT_PORT
import dev.perritodeck.client.protocol.Hello
import dev.perritodeck.client.protocol.Platform
import dev.perritodeck.client.protocol.Screen
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Which top-level screen is showing. */
sealed interface Route {
    data object Home : Route
    data object Pair : Route
    data object Deck : Route
    data object Scan : Route
    data object Licenses : Route
}

/** Where the pairing flow is. */
sealed interface PairState {
    data class Form(val address: String, val error: String? = null) : PairState
    data class Working(val message: String) : PairState
    /**
     * Manual pairing: the user compares this fingerprint with the one the computer shows.
     * [host] and [port] answered (127.0.0.1 over a USB cable); [save] is the address to
     * reconnect to later.
     */
    data class Confirm(
        val host: String,
        val port: Int,
        val code: String,
        val fingerprint: String,
        val save: HostPort,
    ) : PairState
}

class DeckViewModel(app: Application) : AndroidViewModel(app) {
    val settings = Settings(app)
    private val savedAddresses = SavedAddressCheck(PROBE_TIMEOUT_MS) { Pairing.probe(it.host, it.port) }
    val discovery = Discovery(app, onNetworkChange = ::networkChanged)
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

    /**
     * Paired computers answering at their saved address, for Home's "On this network" when
     * discovery can't see them (see [SavedAddressCheck]).
     */
    val reachable: StateFlow<Set<String>> = savedAddresses.reachable

    private var pairJob: Job? = null
    private var homeShowing = false
    private var checkJob: Job? = null

    /** An address discovery saw for a paired host, saved once a connection through it works. */
    private data class SeenAddress(val hostId: String, val address: HostPort)

    private var unconfirmed: SeenAddress? = null

    init {
        // A dedicated deck tablet should come straight back to its computer after a restart.
        settings.lastHostId?.let { id -> _paired.value.firstOrNull { it.hostId == id } }?.let(::connectTo)

        viewModelScope.launch {
            connection.state.collect { s ->
                if (s is ConnState.Connected) confirmAddress(s.url)
                if (s is ConnState.Failed && _route.value == Route.Deck) {
                    _notice.value = s.reason
                    _route.value = Route.Home
                }
            }
        }
    }

    /** Home is showing: look for computers, and check the saved ones' addresses now and then. */
    fun homeShown() {
        homeShowing = true
        discovery.start()
        checkSavedAddresses()
    }

    fun homeHidden() {
        homeShowing = false
        discovery.stop()
        checkJob?.cancel()
    }

    /** A Wi-Fi or Ethernet network came up ([up]) or went away. */
    private fun networkChanged(up: Boolean) {
        if (!up) {
            checkJob?.cancel()
            savedAddresses.clear()
        }
        if (homeShowing) checkSavedAddresses()
    }

    private fun checkSavedAddresses() {
        checkJob?.cancel()
        checkJob = viewModelScope.launch {
            while (true) {
                savedAddresses.check(_paired.value)
                delay(RECHECK_MS)
            }
        }
    }

    fun connectTo(host: PairedHost) {
        // Discovery may have seen the computer at a new address (DHCP moved it). It's tried
        // before the saved one but only saved once it works: older Android can report an
        // address the host doesn't listen on.
        val seen = discovery.hosts.value.firstOrNull { it.hostId == host.hostId }
            ?.let { HostPort(it.address, it.port) }
            ?.takeIf { it != HostPort(host.address, host.port) }
        unconfirmed = seen?.let { SeenAddress(host.hostId, it) }
        settings.lastHostId = host.hostId
        _notice.value = null
        // 127.0.0.1 goes first: over a USB cable the computer forwards its port there (adb
        // reverse), and without one it's refused at once. The certificate pin makes sure
        // whatever answers is the same computer.
        val urls = host.urls(seen)
        connection.connect(
            urls.first(),
            hello(host.deviceSecret),
            Tls.pinnedClient(Tls.defaultClient, host.hostId),
            fallbacks = urls.drop(1),
            // Remembered across restarts, so the computer hears about it over the cable.
            networkFailed = settings.networkFailed(host.hostId),
            onNetworkFailed = { settings.setNetworkFailed(host.hostId, it) },
        )
        _route.value = Route.Deck
    }

    private fun confirmAddress(url: String) {
        val seen = unconfirmed?.takeIf { it.address.url == url } ?: return
        unconfirmed = null
        settings.updateAddress(seen.hostId, seen.address.host, seen.address.port)
        _paired.value = settings.pairedHosts
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

    fun openScan() {
        pairJob?.cancel()
        _route.value = Route.Scan
    }

    fun closePairing() {
        pairJob?.cancel()
        _route.value = Route.Home
    }

    fun openLicenses() {
        _route.value = Route.Licenses
    }

    fun closeLicenses() {
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
                // When discovery saw which computer is at this address, an answer from it
                // over the USB cable needn't wait for a network that drops packets.
                val expected = discovery.hosts.value.firstOrNull { HostPort(it.address, it.port) == hp }?.hostId
                pairJob?.cancel()
                pairJob = viewModelScope.launch {
                    _pairState.value = PairState.Working("Reaching ${hp.host}…")
                    _pairState.value = try {
                        // 127.0.0.1 is tried too, as with a QR code: a USB cable reaches the
                        // computer even when the network doesn't let the tablet through.
                        // The typed address is the one saved: every connect tries the cable
                        // by itself, and unplugged the tablet should still find the computer.
                        val (at, fingerprint) = Pairing.probeWithCable(hp, expected)
                        PairState.Confirm(at.host, at.port, digits, fingerprint, save = hp)
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
        pairJob = viewModelScope.launch {
            finishPairing(c.host, c.port, c.fingerprint, c.code, "${c.save.host}:${c.save.port}", c.save)
        }
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
            // The host lists every address it has, and 127.0.0.1 covers a USB cable; use
            // the first that answers as this computer.
            val addr = Pairing.firstReachable(link.candidates, link.fingerprint)
            if (addr != null) {
                finishPairing(addr.host, addr.port, link.fingerprint, link.code, "${addr.host}:${addr.port}", link.addressToSave(addr))
                return@launch
            }
            _pairState.value = PairState.Form(
                link.addresses.first().let { "${it.host}:${it.port}" },
                "Couldn't reach ${link.hostName} over Wi-Fi or USB. Make sure this tablet is on the same " +
                    "Wi-Fi network and the computer's firewall allows PerritoDeck.",
            )
        }
    }

    /** [save] is the address to reconnect to later, when it isn't the one used to pair. */
    private suspend fun finishPairing(
        host: String,
        port: Int,
        fingerprint: String,
        code: String,
        address: String,
        save: HostPort = HostPort(host, port),
    ) {
        _pairState.value = PairState.Working("Pairing…")
        try {
            val paired = Pairing.pair(Tls.defaultClient, host, port, fingerprint, code, settings.deviceId, deviceName())
                .copy(address = save.host, port = save.port)
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
            capabilities = listOf("keys", "haptics", "multiTouch", "images", "sliders"),
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
        discovery.close()
    }

    private companion object {
        /** A TLS handshake with the saved address; a computer on this network answers well within it. */
        const val PROBE_TIMEOUT_MS = 3_000L

        /** How often Home checks the saved addresses again, so a computer that sleeps drops out. */
        const val RECHECK_MS = 30_000L
    }
}
