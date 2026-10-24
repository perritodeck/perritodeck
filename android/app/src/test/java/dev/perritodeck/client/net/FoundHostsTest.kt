package dev.perritodeck.client.net

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Home says "On this network" for a paired computer only while discovery can see it. The host
 * advertises itself as "PerritoDeck on <computer>" and puts the computer's own name in its "n"
 * TXT record, which is the name shown; NsdManager names the service when it's lost.
 */
class FoundHostsTest {
    private val desktop = FoundHost("DESKTOP-6Q41SNJ", "192.168.1.20", 46710, "fp-desktop")
    private val service = "PerritoDeck on DESKTOP-6Q41SNJ"

    @Test
    fun aHostIsGoneWhenItsServiceIsLost() {
        val found = FoundHosts()
        found.newRun()
        found.resolved(service, desktop)
        assertEquals(listOf(desktop), found.hosts.value)
        found.lost(service)
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }

    @Test
    fun aHostResolvedUnderAnEscapedNameIsStillLost() {
        // Before Android 7 the resolved NsdServiceInfo carries the daemon's DNS-escaped name
        // (the Fire 7 logs SERVICE_RESOLVED "PerritoDeck\032on\032DESKTOP-6Q41SNJ..."), while
        // onServiceFound and onServiceLost report "PerritoDeck on DESKTOP-6Q41SNJ".
        val found = FoundHosts()
        found.newRun()
        found.resolved("PerritoDeck\\032on\\032DESKTOP-6Q41SNJ", desktop)
        assertEquals(listOf(desktop), found.hosts.value)
        found.lost(service)
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }

    @Test
    fun serviceNamesAreUnescaped() {
        assertEquals(service, unescapeServiceName("PerritoDeck\\032on\\032DESKTOP-6Q41SNJ"))
        assertEquals(service, unescapeServiceName(service))
        assertEquals("a.b\\c", unescapeServiceName("a\\.b\\\\c"))
        // Bytes past ASCII are UTF-8, escaped or not.
        assertEquals("PerritoDeck on Café", unescapeServiceName("PerritoDeck\\032on\\032Caf\\195\\169"))
        assertEquals("PerritoDeck on Café", unescapeServiceName("PerritoDeck\\032on\\032Café"))
        // A backslash that starts no complete escape stays.
        assertEquals("odd\\", unescapeServiceName("odd\\"))
        assertEquals("odd\\4x", unescapeServiceName("odd\\4x"))
        assertEquals("odd\\999", unescapeServiceName("odd\\999"))
    }

    @Test
    fun aNewRunForgetsHostsNobodyWasListeningFor() {
        // Discovery stops while the deck is showing. If the computer goes away meanwhile (the
        // tablet's Wi-Fi is switched off), nothing says so: the next run starts from nothing.
        val found = FoundHosts()
        found.newRun()
        found.resolved(service, desktop)
        found.newRun()
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }

    @Test
    fun theSameComputerAtANewAddressReplacesItself() {
        val found = FoundHosts()
        found.newRun()
        found.resolved(service, desktop)
        val moved = desktop.copy(address = "192.168.1.21")
        found.resolved(service, moved)
        assertEquals(listOf(moved), found.hosts.value)
        // Renamed, it advertises under another service name but is still the same computer.
        val renamed = moved.copy(name = "STUDIO")
        found.resolved("PerritoDeck on STUDIO", renamed)
        assertEquals(listOf(renamed), found.hosts.value)
        found.lost("PerritoDeck on STUDIO")
        assertEquals(emptyList<FoundHost>(), found.hosts.value)
    }

    @Test
    fun anotherComputerStays() {
        val found = FoundHosts()
        found.newRun()
        val laptop = FoundHost("laptop", "192.168.1.30", 46710, "fp-laptop")
        found.resolved(service, desktop)
        found.resolved("PerritoDeck on laptop", laptop)
        found.lost(service)
        assertEquals(listOf(laptop), found.hosts.value)
    }
}
