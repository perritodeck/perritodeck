package dev.perritodeck.client

import android.app.Activity
import android.content.Intent
import android.content.pm.ActivityInfo
import android.os.Bundle
import android.view.ViewTreeObserver
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import dev.perritodeck.client.scan.ScanScreen
import dev.perritodeck.client.scan.hasCamera
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.perritodeck.client.ui.DeckScreen
import dev.perritodeck.client.ui.HomeScreen
import dev.perritodeck.client.ui.LicensesScreen
import dev.perritodeck.client.ui.PairScreen
import dev.perritodeck.client.ui.PerritoTheme
import dev.perritodeck.client.ui.orientationFor

class MainActivity : ComponentActivity() {
    private val vm: DeckViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handleIntent(intent)
        setContent {
            PerritoTheme {
                PerritoApp(vm)
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** A scanned pairing QR code opens the app with a perritodeck://pair link. */
    private fun handleIntent(intent: Intent?) {
        val data = intent?.data ?: return
        if (intent.action == Intent.ACTION_VIEW && data.scheme == "perritodeck") vm.handleLink(data.toString())
    }
}

@Composable
private fun PerritoApp(vm: DeckViewModel) {
    val route by vm.route.collectAsStateWithLifecycle()
    when (route) {
        Route.Home -> {
            val context = LocalContext.current
            val canScan = remember { hasCamera(context) }
            val paired by vm.paired.collectAsStateWithLifecycle()
            val found by vm.discovery.hosts.collectAsStateWithLifecycle()
            val reachable by vm.reachable.collectAsStateWithLifecycle()
            val notice by vm.notice.collectAsStateWithLifecycle()
            DisposableEffect(Unit) {
                vm.homeShown()
                onDispose { vm.homeHidden() }
            }
            HomeScreen(
                paired = paired,
                found = found,
                reachable = reachable,
                notice = notice,
                onConnect = vm::connectTo,
                onForget = vm::forget,
                onPairFound = { vm.openPairing("${it.address}:${it.port}") },
                onPairManually = { vm.openPairing() },
                onScan = if (canScan) vm::openScan else null,
                onLicenses = vm::openLicenses,
            )
        }
        Route.Licenses -> {
            BackHandler { vm.closeLicenses() }
            LicensesScreen(onBack = vm::closeLicenses)
        }
        Route.Pair -> {
            val pairState by vm.pairState.collectAsStateWithLifecycle()
            BackHandler { vm.closePairing() }
            PairScreen(
                state = pairState,
                onSubmit = vm::submitCode,
                onConfirm = vm::confirmPairing,
                onCancel = vm::closePairing,
            )
        }
        Route.Scan -> {
            BackHandler { vm.closePairing() }
            ScanScreen(onLink = vm::handleLink, onCancel = vm::closePairing)
        }
        Route.Deck -> {
            val state by vm.connection.state.collectAsStateWithLifecycle()
            val layout by vm.connection.layout.collectAsStateWithLifecycle()
            val images by vm.connection.images.collectAsStateWithLifecycle()
            val flash by vm.connection.flash.collectAsStateWithLifecycle()
            val rtt by vm.connection.rttMs.collectAsStateWithLifecycle()
            DeckMode()
            DeckOrientation(layout?.cols, layout?.rows)
            BackHandler { vm.disconnect() }
            DeckScreen(
                layout = layout,
                images = images,
                flash = flash,
                state = state,
                rttMs = rtt,
                onKeyDown = vm.connection::keyDown,
                onKeyUp = vm.connection::keyUp,
                onDisconnect = vm::disconnect,
                onSlider = vm.connection::sliderChange,
            )
        }
    }
}

/**
 * While the deck is showing, the screen turns to the grid's shape (landscape for 5×3), as
 * the computer's editor draws it. Leaving the deck gives rotation back to the tablet.
 */
@Composable
private fun DeckOrientation(cols: Int?, rows: Int?) {
    val activity = LocalView.current.context as Activity
    val wanted = if (cols != null && rows != null) orientationFor(cols, rows) else null
    LaunchedEffect(wanted) {
        if (wanted != null) activity.requestedOrientation = wanted
    }
    DisposableEffect(activity) {
        onDispose { activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED }
    }
}

/** While the deck is showing: full-screen (bars swipe in temporarily) and screen kept on. */
@Composable
private fun DeckMode() {
    val view = LocalView.current
    val window = (view.context as Activity).window
    val controller = remember(view) { WindowCompat.getInsetsController(window, view) }
    val hideBars = {
        controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
    }
    DisposableEffect(view) {
        hideBars()
        // Before Android 11 (Fire OS 7 is Android 9) hidden bars come back for good once
        // the window loses focus: the lock screen, the notification shade, a dialog.
        val refocus = ViewTreeObserver.OnWindowFocusChangeListener { focused -> if (focused) hideBars() }
        view.viewTreeObserver.addOnWindowFocusChangeListener(refocus)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            view.viewTreeObserver.removeOnWindowFocusChangeListener(refocus)
            controller.show(WindowInsetsCompat.Type.systemBars())
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }
    // Turning the screen (see DeckOrientation) can bring them back too.
    LaunchedEffect(LocalConfiguration.current.orientation) { hideBars() }
}
