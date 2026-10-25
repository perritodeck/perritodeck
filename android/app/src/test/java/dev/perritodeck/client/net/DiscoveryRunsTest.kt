package dev.perritodeck.client.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * When discovery runs, from Home showing and the Wi-Fi or Ethernet networks Android reports.
 * On the Fire 7 (Fire OS 7) NsdManager never said the host was lost while Wi-Fi was off, and
 * a run started before Wi-Fi came back found nothing afterwards: the network itself decides.
 */
class DiscoveryRunsTest {
    private val found = FoundHosts()
    private val log = mutableListOf<String>()
    private val runs = DiscoveryRuns<String>(found, begin = { log += "begin" }, end = { log += "end" })
    private val desktop = FoundHost("DESKTOP-6Q41SNJ", "192.168.1.60", 46720, "fp-desktop")
    private val service = "PerritoDeck on DESKTOP-6Q41SNJ"

    @Test
    fun homeOnANetworkDiscovers() {
        runs.seed(listOf("wifi"))
        runs.start()
        assertEquals(listOf("begin"), log)
        assertTrue(runs.running)
        // Home is shown again (recomposed): one run is enough.
        runs.start()
        assertEquals(listOf("begin"), log)
        runs.stop()
        assertEquals(listOf("begin", "end"), log)
        assertFalse(runs.running)
    }

    @Test
    fun withoutANetworkNothingRunsUntilOneComes() {
        // A run started with Wi-Fi off got a stale cached service whose resolve never answered.
        runs.start()
        assertEquals(emptyList<String>(), log)
        assertTrue(runs.available("wifi"))
        assertEquals(listOf("begin"), log)
    }

    @Test
    fun theFirstReportOfANetworkAlreadyThereChangesNothing() {
        // Registering the callback reports every network already up, right after the snapshot.
        runs.seed(listOf("wifi"))
        runs.start()
        assertFalse(runs.available("wifi"))
        assertEquals(listOf("begin"), log)
    }

    @Test
    fun losingTheNetworkForgetsWhatWasFoundAtOnce() {
        runs.seed(listOf("wifi"))
        runs.start()
        found.resolved(service, desktop)
        assertTrue(runs.lost("wifi"))
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
        assertEquals(listOf("begin", "end"), log)
        assertFalse(runs.running)
        // Wi-Fi back: a new run, not the old one, which may be deaf.
        assertTrue(runs.available("wifi-again"))
        assertEquals(listOf("begin", "end", "begin"), log)
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }

    @Test
    fun aNetworkLostWhileTheDeckShowsIsForgottenToo() {
        runs.seed(listOf("wifi"))
        runs.start()
        found.resolved(service, desktop)
        runs.stop()
        assertTrue(runs.lost("wifi"))
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
        // Not on Home: nothing starts when Wi-Fi is back, only when Home is shown.
        runs.available("wifi")
        assertEquals(listOf("begin", "end"), log)
        runs.start()
        assertEquals(listOf("begin", "end", "begin"), log)
    }

    @Test
    fun aNewNetworkStartsOver() {
        // Moving to another Wi-Fi network: Android can report the new one before the old one's
        // loss. What the old run found may not be on the new network.
        runs.seed(listOf("home"))
        runs.start()
        found.resolved(service, desktop)
        assertTrue(runs.available("office"))
        assertEquals(listOf("begin", "end", "begin"), log)
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
        found.resolved(service, desktop)
        // The old network's loss: what this run found may have come through it.
        assertTrue(runs.lost("home"))
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
        assertEquals(listOf("begin", "end", "begin", "end", "begin"), log)
        assertTrue(runs.running)
    }

    @Test
    fun aNetworkNeverReportedIsNoChange() {
        runs.seed(listOf("wifi"))
        runs.start()
        found.resolved(service, desktop)
        assertFalse(runs.lost("cellular"))
        assertEquals(listOf(desktop), found.hosts.value)
        assertEquals(listOf("begin"), log)
    }

    @Test
    fun aRunThatFailedToStartIsTriedAgain() {
        runs.seed(listOf("wifi"))
        runs.start()
        runs.beginFailed()
        assertFalse(runs.running)
        // Nothing to stop.
        runs.stop()
        assertEquals(listOf("begin"), log)
        runs.start()
        assertEquals(listOf("begin", "begin"), log)
        runs.beginFailed()
        runs.available("ethernet")
        assertEquals(listOf("begin", "begin", "begin"), log)
    }

    @Test
    fun eachRunStartsFromNothing() {
        runs.seed(listOf("wifi"))
        runs.start()
        found.resolved(service, desktop)
        runs.stop()
        runs.start()
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }
}
