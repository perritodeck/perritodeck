package dev.perritodeck.client.net

import dev.perritodeck.client.data.HostPort
import dev.perritodeck.client.data.PairedHost
import dev.perritodeck.client.data.isLoopback
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * The paired computers that answered at their saved address as themselves, for Home's "On
 * this network" when discovery can't see them: after Wi-Fi goes off and on, the Fire 7's
 * mdnsd hears nothing until the app's process ends. [probe] returns the certificate
 * fingerprint the address shows (a TLS handshake, nothing sent); [timeoutMs] bounds it.
 * Used on the main thread.
 */
internal class SavedAddressCheck(
    private val timeoutMs: Long,
    private val probe: suspend (HostPort) -> String,
) {
    private val _reachable = MutableStateFlow<Set<String>>(emptySet())
    val reachable: StateFlow<Set<String>> = _reachable.asStateFlow()

    /** Bumped by [clear], so a check that was under way when the network went counts for nothing. */
    private var generation = 0

    /**
     * Probes every host's saved address at once; each counts as soon as it answers and stops
     * counting when it doesn't. 127.0.0.1 is the USB cable, not the network, so it isn't tried.
     */
    suspend fun check(hosts: List<PairedHost>) {
        val gen = generation
        val ids = hosts.map { it.hostId }.toSet()
        _reachable.value = _reachable.value intersect ids
        coroutineScope {
            hosts.filterNot { isLoopback(it.address) }.forEach { host ->
                launch {
                    val answers = answersAs(HostPort(host.address, host.port)) == host.hostId
                    if (gen == generation) {
                        _reachable.value = if (answers) _reachable.value + host.hostId else _reachable.value - host.hostId
                    }
                }
            }
        }
    }

    /** The network went away: nothing is reachable until a new check says so. */
    fun clear() {
        generation++
        _reachable.value = emptySet()
    }

    private suspend fun answersAs(at: HostPort): String? = try {
        withTimeoutOrNull(timeoutMs) { probe(at) }
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        null // refused, unreachable, or not PerritoDeck
    }
}

/** The paired computers Home shows as "On this network": seen by discovery, or answering at their saved address. */
internal fun onThisNetwork(found: List<FoundHost>, reachable: Set<String>): Set<String> =
    found.mapNotNull { it.hostId }.toSet() + reachable
