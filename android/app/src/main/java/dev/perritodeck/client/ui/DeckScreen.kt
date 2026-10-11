package dev.perritodeck.client.ui

import android.view.HapticFeedbackConstants
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import dev.perritodeck.client.R
import dev.perritodeck.client.net.ConnState
import dev.perritodeck.client.protocol.KeySpec
import dev.perritodeck.client.protocol.Layout
import dev.perritodeck.client.protocol.parseColor

@Composable
fun DeckScreen(
    layout: Layout?,
    state: ConnState,
    rttMs: Double?,
    onKeyDown: (Int) -> Unit,
    onKeyUp: (Int) -> Unit,
    onDisconnect: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxSize()
            .background(Background)
    ) {
        if (layout != null) {
            KeyGrid(layout, onKeyDown, onKeyUp)
        }

        // Small status line; stays out of the way of the grid.
        if (state is ConnState.Connected && rttMs != null) {
            Text(
                text = stringResource(R.string.latency, state.welcome.hostName, rttMs),
                color = Muted,
                style = androidx.compose.material3.MaterialTheme.typography.labelSmall,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(horizontal = 8.dp, vertical = 2.dp),
            )
        }

        if (state !is ConnState.Connected || layout == null) {
            ConnectingOverlay(state, onDisconnect)
        }
    }
}

@Composable
private fun ConnectingOverlay(state: ConnState, onDisconnect: () -> Unit) {
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.6f)),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator(color = Brand)
            Spacer(Modifier.height(16.dp))
            val text = when (state) {
                is ConnState.Connecting -> state.lastError?.let { stringResource(R.string.reconnecting, it) }
                    ?: stringResource(R.string.connecting, state.url)
                else -> ""
            }
            Text(text, color = OnSurface, textAlign = TextAlign.Center, modifier = Modifier.padding(horizontal = 24.dp))
            Spacer(Modifier.height(8.dp))
            TextButton(onClick = onDisconnect) { Text(stringResource(R.string.disconnect)) }
        }
    }
}

@Composable
private fun KeyGrid(layout: Layout, onKeyDown: (Int) -> Unit, onKeyUp: (Int) -> Unit) {
    val byIndex = remember(layout) { layout.keys.associateBy { it.index } }
    BoxWithConstraints(
        Modifier
            .fillMaxSize()
            .padding(8.dp),
        contentAlignment = Alignment.Center,
    ) {
        val gap = 8.dp
        val cols = layout.cols.coerceAtLeast(1)
        val rows = layout.rows.coerceAtLeast(1)
        val cell: Dp = min(
            (maxWidth - gap * (cols - 1)) / cols,
            (maxHeight - gap * (rows - 1)) / rows,
        ).coerceAtLeast(16.dp)

        Column {
            for (r in 0 until rows) {
                if (r > 0) Spacer(Modifier.height(gap))
                Row {
                    for (c in 0 until cols) {
                        if (c > 0) Spacer(Modifier.size(gap))
                        val index = r * cols + c
                        DeckKey(index, byIndex[index], cell, onKeyDown, onKeyUp)
                    }
                }
            }
        }
    }
}

@Composable
private fun DeckKey(
    index: Int,
    spec: KeySpec?,
    size: Dp,
    onKeyDown: (Int) -> Unit,
    onKeyUp: (Int) -> Unit,
) {
    val view = LocalView.current
    var pressed by remember { mutableStateOf(false) }
    val down by rememberUpdatedState(onKeyDown)
    val up by rememberUpdatedState(onKeyUp)

    val bg = spec?.bgColor?.let(::parseColor)?.let(::Color) ?: Surface
    val fg = spec?.textColor?.let(::parseColor)?.let(::Color) ?: OnSurface
    val title = spec?.title.orEmpty()
    val fontSize = with(LocalDensity.current) { (size.toPx() * 0.16f).toSp() }
    val description = stringResource(R.string.key_description, index + 1, title)

    Box(
        modifier = Modifier
            .size(size)
            .scale(if (pressed) 0.94f else 1f)
            .clip(RoundedCornerShape(size * 0.14f))
            .background(if (pressed) bg.copy(alpha = 0.7f) else bg)
            .semantics { contentDescription = description }
            // Each key tracks its own pointer, so several keys can be held at once.
            .pointerInput(index) {
                awaitEachGesture {
                    val first = awaitFirstDown(requireUnconsumed = false)
                    first.consume()
                    pressed = true
                    view.performHapticFeedback(HapticFeedbackConstants.KEYBOARD_TAP)
                    down(index)
                    try {
                        waitForUpOrCancellation()?.consume()
                    } finally {
                        pressed = false
                        up(index)
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = title,
            color = fg,
            fontSize = fontSize,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
            lineHeight = fontSize * 1.1f,
            modifier = Modifier.padding(size * 0.06f),
        )
    }
}
