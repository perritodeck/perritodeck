package dev.perritodeck.client

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.perritodeck.client.ui.DeckScreen
import dev.perritodeck.client.ui.HomeScreen
import dev.perritodeck.client.ui.PairScreen
import dev.perritodeck.client.ui.PerritoTheme

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
            val paired by vm.paired.collectAsStateWithLifecycle()
            val found by vm.discovery.hosts.collectAsStateWithLifecycle()
            val notice by vm.notice.collectAsStateWithLifecycle()
            DisposableEffect(Unit) {
                vm.discovery.start()
                onDispose { vm.discovery.stop() }
            }
            HomeScreen(
                paired = paired,
                found = found,
                notice = notice,
                onConnect = vm::connectTo,
                onForget = vm::forget,
                onPairFound = { vm.openPairing("${it.address}:${it.port}") },
                onPairManually = { vm.openPairing() },
            )
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
        Route.Deck -> {
            val state by vm.connection.state.collectAsStateWithLifecycle()
            val layout by vm.connection.layout.collectAsStateWithLifecycle()
            val images by vm.connection.images.collectAsStateWithLifecycle()
            val flash by vm.connection.flash.collectAsStateWithLifecycle()
            val rtt by vm.connection.rttMs.collectAsStateWithLifecycle()
            DeckMode()
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
            )
        }
    }
}

/** While the deck is showing: full-screen (bars swipe in temporarily) and screen kept on. */
@Composable
private fun DeckMode() {
    val view = LocalView.current
    DisposableEffect(view) {
        val window = (view.context as Activity).window
        val controller = WindowCompat.getInsetsController(window, view)
        controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        onDispose {
            controller.show(WindowInsetsCompat.Type.systemBars())
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }
}
