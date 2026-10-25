package dev.perritodeck.client.net

import dev.perritodeck.client.data.HostPort
import dev.perritodeck.client.data.PairedHost
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test
import java.net.ConnectException

/**
 * After Wi-Fi goes off and on, the Fire 7's mdnsd hears nothing until the app's process
 * ends, so a paired computer is also "On this network" when its saved address answers as it.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SavedAddressCheckTest {
    private val desktop = PairedHost("fp-desktop", "DESKTOP-6Q41SNJ", "192.168.1.60", 46720, "secret")
    private val laptop = PairedHost("fp-laptop", "laptop", "192.168.1.30", 46710, "secret")

    @Test
    fun aComputerThatAnswersAsItselfIsOnThisNetwork() = runTest {
        val probed = mutableListOf<HostPort>()
        val check = SavedAddressCheck(TIMEOUT) { at -> probed += at; "fp-desktop" }
        check.check(listOf(desktop))
        assertEquals(setOf("fp-desktop"), check.reachable.value)
        assertEquals(listOf(HostPort("192.168.1.60", 46720)), probed)
    }

    @Test
    fun anotherComputerAtThatAddressDoesNotCount() = runTest {
        // DHCP gave the address to another computer running PerritoDeck.
        val check = SavedAddressCheck(TIMEOUT) { "fp-someone-else" }
        check.check(listOf(desktop))
        assertEquals(emptySet<String>(), check.reachable.value)
    }

    @Test
    fun noAnswerOrNoNetworkIsNotOnThisNetwork() = runTest {
        var answer: suspend () -> String = { "fp-desktop" }
        val check = SavedAddressCheck(TIMEOUT) { answer() }
        check.check(listOf(desktop))
        assertEquals(setOf("fp-desktop"), check.reachable.value)
        // Refused (the computer's asleep, or there's no network to try)...
        answer = { throw ConnectException("ENETUNREACH") }
        check.check(listOf(desktop))
        assertEquals(emptySet<String>(), check.reachable.value)
        // ...or dropped packets, until the timeout.
        answer = { "fp-desktop" }
        check.check(listOf(desktop))
        answer = { awaitCancellation() }
        val start = testScheduler.currentTime
        check.check(listOf(desktop))
        assertEquals(TIMEOUT, testScheduler.currentTime - start)
        assertEquals(emptySet<String>(), check.reachable.value)
    }

    @Test
    fun theCableDoesNotCount() = runTest {
        // Paired over USB with no other address: 127.0.0.1 answers whenever the cable is in.
        val probed = mutableListOf<HostPort>()
        val check = SavedAddressCheck(TIMEOUT) { at -> probed += at; "fp-desktop" }
        check.check(listOf(desktop.copy(address = "127.0.0.1")))
        assertEquals(emptyList<HostPort>(), probed)
        assertEquals(emptySet<String>(), check.reachable.value)
    }

    @Test
    fun losingTheNetworkForgetsAtOnce() = runTest {
        val check = SavedAddressCheck(TIMEOUT) { "fp-desktop" }
        check.check(listOf(desktop))
        check.clear()
        assertEquals(emptySet<String>(), check.reachable.value)
    }

    @Test
    fun aCheckThatEndsAfterTheNetworkWentAwayChangesNothing() = runTest {
        val answer = CompletableDeferred<String>()
        val check = SavedAddressCheck(TIMEOUT) { answer.await() }
        val job = launch { check.check(listOf(desktop)) }
        runCurrent()
        check.clear()
        answer.complete("fp-desktop")
        job.join()
        assertEquals(emptySet<String>(), check.reachable.value)
    }

    @Test
    fun eachComputerCountsAsSoonAsItAnswers() = runTest {
        val check = SavedAddressCheck(TIMEOUT) { at ->
            if (at.host == laptop.address) awaitCancellation() else "fp-desktop"
        }
        val job = launch { check.check(listOf(laptop, desktop)) }
        runCurrent()
        assertEquals(setOf("fp-desktop"), check.reachable.value)
        advanceTimeBy(TIMEOUT + 1)
        job.join()
        assertEquals(setOf("fp-desktop"), check.reachable.value)
    }

    @Test
    fun aForgottenComputerDropsOut() = runTest {
        val check = SavedAddressCheck(TIMEOUT) { at -> if (at.host == laptop.address) "fp-laptop" else "fp-desktop" }
        check.check(listOf(desktop, laptop))
        assertEquals(setOf("fp-desktop", "fp-laptop"), check.reachable.value)
        check.check(listOf(desktop))
        assertEquals(setOf("fp-desktop"), check.reachable.value)
    }

    @Test
    fun homeCombinesDiscoveryAndTheSavedAddresses() {
        val seen = FoundHost("DESKTOP-6Q41SNJ", "192.168.1.60", 46720, "fp-desktop")
        val unpaired = FoundHost("old", "192.168.1.9", 46710, null)
        assertEquals(setOf("fp-desktop"), onThisNetwork(listOf(seen, unpaired), emptySet()))
        assertEquals(setOf("fp-laptop"), onThisNetwork(emptyList(), setOf("fp-laptop")))
        assertEquals(setOf("fp-desktop", "fp-laptop"), onThisNetwork(listOf(seen), setOf("fp-laptop")))
    }

    private companion object {
        const val TIMEOUT = 3_000L
    }
}
