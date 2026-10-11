package dev.perritodeck.client

import android.app.Activity
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
import dev.perritodeck.client.net.ConnState
import dev.perritodeck.client.ui.ConnectScreen
import dev.perritodeck.client.ui.DeckScreen
import dev.perritodeck.client.ui.PerritoTheme

class MainActivity : ComponentActivity() {
    private val vm: DeckViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            PerritoTheme {
                PerritoApp(vm)
            }
        }
    }
}

@Composable
private fun PerritoApp(vm: DeckViewModel) {
    val state by vm.connection.state.collectAsStateWithLifecycle()
    val layout by vm.connection.layout.collectAsStateWithLifecycle()
    val rtt by vm.connection.rttMs.collectAsStateWithLifecycle()

    when (val s = state) {
        ConnState.Idle, is ConnState.Failed -> ConnectScreen(
            initial = vm.settings.lastTarget,
            error = (s as? ConnState.Failed)?.reason,
            onConnect = vm::connect,
        )
        is ConnState.Connecting, is ConnState.Connected -> {
            DeckMode()
            BackHandler { vm.disconnect() }
            DeckScreen(
                layout = layout,
                state = s,
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
