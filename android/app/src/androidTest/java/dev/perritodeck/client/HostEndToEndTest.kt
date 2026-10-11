package dev.perritodeck.client

import android.graphics.Bitmap
import android.os.ParcelFileDescriptor
import android.util.Log
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.lifecycle.ViewModelProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.perritodeck.client.net.ConnState
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Runs the real app against a real `perritodeck-host` started by CI on the machine
 * running the emulator (reachable as 10.0.2.2) with a fixed pairing code. Proves the
 * whole loop: pair over TLS, compare fingerprints, connect, then use the host's starter
 * profile — open a page, run a command, go back and flip a toggle. Then the things a
 * tablet left on a desk runs into: the screen sleeping, and the network dropping out.
 * Press latency is logged with the tag `PerritoE2E`.
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

        measureLatency()

        // The emulator's Android 5.1 image doesn't come back reliably from sleep or a
        // radio switched off (the test framework, not the app, loses the device), so
        // those checks run on Android 6 and later: the Fire 7 is on Fire OS 6 or 7.
        if (android.os.Build.VERSION.SDK_INT < 23) return

        // Screen off for a while, then on: the deck is still there and still works.
        shell("input keyevent KEYCODE_SLEEP")
        Thread.sleep(15_000)
        shell("input keyevent KEYCODE_WAKEUP")
        unlock()
        waitForState("connected after sleep") { it is ConnState.Connected }
        compose.onNode(hasContentDescription("muted", substring = true)).performClick()
        compose.waitUntilAtLeastOneExists(hasContentDescription("live", substring = true), timeout)
        screenshot("5-after-sleep")

        // Network drops out (Wi-Fi off, router reboot): the app keeps retrying, then
        // reconnects on its own once the network is back.
        shell("svc wifi disable")
        shell("svc data disable")
        waitForState("noticed the network went away") { it is ConnState.Connecting }
        screenshot("6-network-down")
        Thread.sleep(5_000)
        shell("svc wifi enable")
        shell("svc data enable")
        waitForState("reconnected after the network came back", 60_000) { it is ConnState.Connected }
        compose.onNode(hasContentDescription("live", substring = true)).performClick()
        compose.waitUntilAtLeastOneExists(hasContentDescription("muted", substring = true), timeout)
        screenshot("7-reconnected")
    }

    /** Get past the emulator's swipe lock screen until the app is in front again. */
    private fun unlock() {
        repeat(10) {
            if (resumed()) return
            shell("wm dismiss-keyguard") // API 23+
            shell("input keyevent 82") // menu key unlocks the swipe keyguard on older versions
            shell("input swipe 300 900 300 100")
            Thread.sleep(1_000)
        }
        throw AssertionError("the app didn't come back after waking the screen")
    }

    private fun resumed(): Boolean = compose.runOnUiThread {
        compose.activity.lifecycle.currentState.isAtLeast(androidx.lifecycle.Lifecycle.State.RESUMED)
    }

    private val vm: DeckViewModel by lazy {
        compose.runOnUiThread { ViewModelProvider(compose.activity)[DeckViewModel::class.java] }
    }

    /** Press a key repeatedly and log the round trip the app measures for each ack. */
    private fun measureLatency() {
        val key = compose.onNode(hasContentDescription("Copy", substring = true))
        val samples = mutableListOf<Double>()
        repeat(30) {
            key.performClick()
            Thread.sleep(150)
            vm.connection.rttMs.value?.let(samples::add)
        }
        assertTrue("no round trips measured", samples.isNotEmpty())
        samples.sort()
        val median = samples[samples.size / 2]
        val p95 = samples[((samples.size - 1) * 95) / 100]
        Log.i(
            "PerritoE2E",
            "latency ms: median=%.1f p95=%.1f max=%.1f n=%d".format(median, p95, samples.last(), samples.size),
        )
    }

    private fun waitForState(what: String, ms: Long = timeout, ok: (ConnState) -> Boolean) {
        val until = System.currentTimeMillis() + ms
        while (System.currentTimeMillis() < until) {
            if (ok(vm.connection.state.value)) return
            Thread.sleep(200)
        }
        throw AssertionError("timed out waiting: $what (state ${vm.connection.state.value})")
    }

    /** Runs a shell command as the shell user and waits for it to finish. */
    private fun shell(cmd: String) {
        val pfd = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(cmd)
        ParcelFileDescriptor.AutoCloseInputStream(pfd).use { it.readBytes() }
        Log.i("PerritoE2E", "shell: $cmd")
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
