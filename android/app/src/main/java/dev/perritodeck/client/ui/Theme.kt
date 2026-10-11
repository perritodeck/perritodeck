package dev.perritodeck.client.ui

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

val Background = Color(0xFF11111B)
val SurfaceColor = Color(0xFF1E1E2E)
val Brand = Color(0xFFFAB387)
val OnSurface = Color(0xFFCDD6F4)
val Muted = Color(0xFF9399B2)
val ErrorColor = Color(0xFFF38BA8)

@Composable
fun PerritoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = Brand,
            onPrimary = Background,
            background = Background,
            onBackground = OnSurface,
            surface = SurfaceColor,
            onSurface = OnSurface,
            error = ErrorColor,
        ),
    ) {
        // Surface sets the default text colour; without it headlines render black on the dark background.
        Surface(modifier = Modifier.fillMaxSize(), color = Background, contentColor = OnSurface, content = content)
    }
}
