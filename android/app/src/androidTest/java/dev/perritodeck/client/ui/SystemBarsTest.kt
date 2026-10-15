package dev.perritodeck.client.ui

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.unit.dp
import androidx.core.view.WindowCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.perritodeck.client.PairState
import dev.perritodeck.client.data.PairedHost
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Android 15 and newer draw every app edge to edge (targetSdk 35+): the status and navigation
 * bars sit over the app, which has to keep its content clear of them. These tests make the
 * window edge to edge on whatever version the emulator runs, then check that each screen keeps
 * its 24 dp margin inside the bars.
 */
@RunWith(AndroidJUnit4::class)
class SystemBarsTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val margin get() = with(compose.density) { 24.dp.toPx() }
    private var statusBar = 0
    private var navigationBar = 0

    /** Shows [content] edge to edge, as Android 15 does, and notes the bars' sizes in pixels. */
    private fun show(content: @Composable () -> Unit) {
        compose.runOnUiThread { WindowCompat.setDecorFitsSystemWindows(compose.activity.window, false) }
        compose.setContent {
            val density = LocalDensity.current
            statusBar = WindowInsets.safeDrawing.getTop(density)
            navigationBar = WindowInsets.safeDrawing.getBottom(density)
            PerritoTheme(content)
        }
        compose.waitForIdle()
        assumeTrue("The window got no insets for the status bar, so there's nothing to check.", statusBar > 0)
    }

    private fun assertBelowTheStatusBar(node: SemanticsNodeInteraction) {
        val top = node.fetchSemanticsNode().boundsInRoot.top
        assertTrue("top at $top px, under the $statusBar px status bar plus the margin", top >= statusBar + margin - 1)
    }

    private fun assertAboveTheNavigationBar(node: SemanticsNodeInteraction) {
        val bottom = node.fetchSemanticsNode().boundsInRoot.bottom
        val screen = compose.onRoot().fetchSemanticsNode().size.height
        assertTrue("bottom at $bottom px, under the $navigationBar px navigation bar", bottom <= screen - navigationBar + 1)
    }

    @Test
    fun homeScreen() {
        // Enough computers that the screen scrolls.
        val hosts = (1..20).map { PairedHost("id$it", "Computer $it", "192.168.1.$it", 46710, "secret") }
        show {
            HomeScreen(
                paired = hosts,
                found = emptyList(),
                notice = null,
                onConnect = {},
                onForget = {},
                onPairFound = {},
                onPairManually = {},
                onScan = null,
                onLicenses = {},
            )
        }
        assertBelowTheStatusBar(compose.onNodeWithText("Perrito"))
        // At the end of the scrolling, the last button is above the navigation bar.
        val licences = compose.onNodeWithText("Open-source licences")
        licences.performScrollTo()
        assertAboveTheNavigationBar(licences)
    }

    @Test
    fun licencesScreen() {
        show { LicensesScreen(onBack = {}) }
        assertBelowTheStatusBar(compose.onNodeWithText("Back"))
    }

    @Test
    fun pairScreen() {
        show { PairScreen(state = PairState.Form(""), onSubmit = { _, _ -> }, onConfirm = {}, onCancel = {}) }
        assertBelowTheStatusBar(compose.onNodeWithText("Pair with your computer"))
    }
}
