package dev.perritodeck.client.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Discovery resolves found services one at a time. On the Fire 7 a resolve of a service
 * found while Wi-Fi was off never called back, and every later resolve waited behind it
 * until the app was killed: each one gives up after a timeout now.
 */
class ResolveQueueTest {
    /** A main-thread Handler's postDelayed, with time moved by hand. */
    private class Clock {
        private class Timer(val at: Long, val action: () -> Unit)
        private val timers = mutableListOf<Timer>()
        var now = 0L
            private set

        fun after(delayMs: Long, action: () -> Unit): () -> Unit {
            val t = Timer(now + delayMs, action)
            timers += t
            return { timers.remove(t) }
        }

        fun advance(ms: Long) {
            now += ms
            while (true) {
                val due = timers.filter { it.at <= now }.minByOrNull { it.at } ?: break
                timers.remove(due)
                due.action()
            }
        }
    }

    private val clock = Clock()
    private val started = mutableListOf<Pair<Long, String>>()
    private val abandoned = mutableListOf<String>()
    private val queue = ResolveQueue<String>(
        timeoutMs = TIMEOUT,
        after = clock::after,
        start = { id, item -> started += id to item },
        abandon = { _, item -> abandoned += item },
    )

    private fun startedItems() = started.map { it.second }
    private fun idOf(item: String) = started.last { it.second == item }.first

    @Test
    fun oneResolveAtATime() {
        queue.add("desktop")
        queue.add("laptop")
        assertEquals(listOf("desktop"), startedItems())
        assertTrue(queue.finished(idOf("desktop")))
        assertEquals(listOf("desktop", "laptop"), startedItems())
        assertTrue(queue.finished(idOf("laptop")))
        assertFalse(queue.busy)
        assertEquals(emptyList<String>(), abandoned)
    }

    @Test
    fun aResolveThatNeverAnswersIsGivenUp() {
        queue.add("stale")
        queue.add("desktop")
        clock.advance(TIMEOUT - 1)
        assertEquals(listOf("stale"), startedItems())
        clock.advance(1)
        assertEquals(listOf("stale"), abandoned)
        assertEquals(listOf("stale", "desktop"), startedItems())
    }

    @Test
    fun anAnswerAfterTheTimeoutIsIgnored() {
        queue.add("stale")
        queue.add("desktop")
        clock.advance(TIMEOUT)
        // The late answer is not for the resolve now waited on, which carries on.
        assertFalse(queue.finished(idOf("stale")))
        assertTrue(queue.busy)
        assertTrue(queue.finished(idOf("desktop")))
    }

    @Test
    fun anAnsweredResolveHasNoTimeoutLeft() {
        queue.add("desktop")
        queue.finished(idOf("desktop"))
        clock.advance(TIMEOUT * 2)
        assertEquals(emptyList<String>(), abandoned)
    }

    @Test
    fun clearEndsTheResolveInFlight() {
        // Discovery stops (Home is left, or the network is lost) during a resolve that will
        // never answer: the next run mustn't wait for it.
        queue.add("stale")
        queue.add("queued")
        clock.advance(TIMEOUT / 2)
        queue.clear()
        assertEquals(listOf("stale"), abandoned)
        assertFalse(queue.busy)
        queue.add("desktop")
        assertEquals(listOf("stale", "desktop"), startedItems())
        // Neither the old resolve's answer nor its timer touches the new one.
        assertFalse(queue.finished(idOf("stale")))
        clock.advance(TIMEOUT / 2 + 1)
        assertEquals(listOf("stale"), abandoned)
        assertTrue(queue.busy)
        assertTrue(queue.finished(idOf("desktop")))
    }

    @Test
    fun clearWithNothingInFlightAbandonsNothing() {
        queue.clear()
        queue.add("desktop")
        queue.finished(idOf("desktop"))
        queue.clear()
        assertEquals(emptyList<String>(), abandoned)
    }

    @Test
    fun aResolveThatFailsAtOnceLetsTheNextGo() {
        // resolveService can throw, and Discovery then finishes the resolve before start returns.
        lateinit var failing: ResolveQueue<String>
        failing = ResolveQueue(
            timeoutMs = TIMEOUT,
            after = clock::after,
            start = { id, item ->
                started += id to item
                if (item == "broken") assertTrue(failing.finished(id))
            },
        )
        failing.add("broken")
        assertFalse(failing.busy)
        failing.add("desktop")
        assertEquals(listOf("broken", "desktop"), startedItems())
        assertTrue(failing.busy)
    }

    private companion object {
        const val TIMEOUT = 8_000L
    }
}
