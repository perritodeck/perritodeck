package dev.perritodeck.client

import android.graphics.Bitmap
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Runs the real app against a real `perritodeck-host` started by CI on the machine
 * running the emulator (reachable as 10.0.2.2) with a fixed pairing code. Proves the
 * whole loop: pair over TLS, compare fingerprints, connect, then use the host's starter
 * profile — open a page, run a command, go back and flip a toggle.
 *
 * Instrumentation args: `pairCode` (default 424242) and optionally `hostFingerprint`
 * (the short form the host printed), which the on-screen fingerprint must equal.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(AndroidJUnit4::class)
class HostEndToEndTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private val timeout = 30_000L
    private val args = InstrumentationRegistry.getArguments()

    @Test
    fun pairConnectAndUseTheStarterProfile() {
        compose.onNodeWithText("Pair with a code").performClick()
        compose.onNodeWithText("Emulator").performClick()
        compose.onNodeWithTag("code").performTextInput(args.getString("pairCode") ?: "424242")
        compose.onNodeWithText("Continue").performClick()

        // The tablet shows the host's fingerprint; it must match what the host printed.
        compose.waitUntilAtLeastOneExists(hasTestTag("fingerprint"), timeout)
        args.getString("hostFingerprint")?.let { compose.onNodeWithTag("fingerprint").assertTextEquals(it) }
        screenshot("1-confirm-fingerprint")
        compose.onNodeWithText("It matches").performClick()

        // Paired and connected over TLS: the host's starter profile is on screen.
        // (Keys may be host-rendered images, so match their accessibility labels, not visible text.)
        compose.waitUntilAtLeastOneExists(hasContentDescription("Copy", substring = true), timeout)
        compose.waitUntilDoesNotExist(hasContentDescription("{time}", substring = true), timeout)
        screenshot("2-connected")

        // A folder key opens the second page; Back returns.
        compose.onNode(hasContentDescription("More", substring = true)).performClick()
        compose.waitUntilAtLeastOneExists(hasContentDescription("Back", substring = true), timeout)

        // Running a command on the host: the key flashes ✓ when it finishes.
        compose.onNode(hasContentDescription("command", substring = true)).performClick()
        Thread.sleep(400)
        screenshot("3-command-ran")

        compose.onNode(hasContentDescription("Back", substring = true)).performClick()
        compose.waitUntilAtLeastOneExists(hasContentDescription("Copy", substring = true), timeout)

        // A toggle switches state on each press (shared by every tablet showing it).
        compose.onNode(hasContentDescription("live", substring = true)).performClick()
        compose.waitUntilAtLeastOneExists(hasContentDescription("muted", substring = true), timeout)
        screenshot("4-toggled")
    }

    /**
     * Best effort: saves to the app's internal files dir; CI copies them out with
     * `adb run-as` (the debug app stays installed after the run). Never fails the test.
     */
    private fun screenshot(name: String) {
        runCatching {
            val instrumentation = InstrumentationRegistry.getInstrumentation()
            instrumentation.waitForIdleSync()
            val bitmap = instrumentation.uiAutomation.takeScreenshot() ?: return
            val dir = File(instrumentation.targetContext.filesDir, "e2e-screenshots").apply { mkdirs() }
            File(dir, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        }
    }
}
