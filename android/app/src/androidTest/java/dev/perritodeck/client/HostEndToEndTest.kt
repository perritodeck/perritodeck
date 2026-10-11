package dev.perritodeck.client

import android.graphics.Bitmap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Runs the real app against a real `perritodeck-host` (started by CI on the machine
 * running the emulator, reachable as 10.0.2.2). Proves the whole loop: connect,
 * receive layout, press a key, see the host's setTitle come back.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(AndroidJUnit4::class)
class HostEndToEndTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private val timeout = 20_000L

    @Test
    fun connectPressAndSeeHostFeedback() {
        compose.onNodeWithText("Emulator").performClick()
        compose.onNodeWithText("Connect").performClick()

        // Layout arrived: the host's demo counter key is on screen.
        compose.waitUntilAtLeastOneExists(hasText("0×", substring = true), timeout)
        screenshot("1-connected")

        // Live clock: the host pushes setTitle every second, replacing the placeholder.
        compose.waitUntilDoesNotExist(hasText("--:--:--"), timeout)

        // Press the counter key three times; the host answers each press with a new title.
        repeat(3) { n ->
            compose.onNodeWithText("Pressed", substring = true).performClick()
            compose.waitUntilAtLeastOneExists(hasText("${n + 1}×", substring = true), timeout)
        }
        screenshot("2-after-presses")
    }

    /** Saves to the app's external files dir; CI pulls the folder with adb. */
    private fun screenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.waitForIdleSync()
        val bitmap = instrumentation.uiAutomation.takeScreenshot() ?: return
        val dir = File(instrumentation.targetContext.getExternalFilesDir(null), "e2e-screenshots").apply { mkdirs() }
        File(dir, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }
}
