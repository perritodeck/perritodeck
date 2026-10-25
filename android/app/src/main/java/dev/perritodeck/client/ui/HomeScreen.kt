package dev.perritodeck.client.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import dev.perritodeck.client.R
import dev.perritodeck.client.data.PairedHost
import dev.perritodeck.client.net.FoundHost
import dev.perritodeck.client.net.onThisNetwork

@Composable
fun HomeScreen(
    paired: List<PairedHost>,
    found: List<FoundHost>,
    /** Paired computers that answered at their saved address. */
    reachable: Set<String>,
    notice: String?,
    onConnect: (PairedHost) -> Unit,
    onForget: (PairedHost) -> Unit,
    onPairFound: (FoundHost) -> Unit,
    onPairManually: () -> Unit,
    onScan: (() -> Unit)?,
    onLicenses: () -> Unit,
) {
    val pairedIds = paired.map { it.hostId }.toSet()
    val unpairedFound = found.filter { it.hostId == null || it.hostId !in pairedIds }
    val onlineIds = onThisNetwork(found, reachable)

    Column(
        Modifier
            .fillMaxSize()
            // Clear of the status and navigation bars, which Android 15 draws over the app.
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 560.dp).fillMaxWidth()) {
            Text(stringResource(R.string.app_name), style = MaterialTheme.typography.headlineLarge)
            Spacer(Modifier.height(4.dp))
            Text(stringResource(R.string.home_subtitle), color = Muted)

            notice?.let {
                Spacer(Modifier.height(16.dp))
                Text(it, color = MaterialTheme.colorScheme.error)
            }

            if (paired.isNotEmpty()) {
                Section(stringResource(R.string.home_your_computers))
                paired.forEach { host ->
                    HostRow(
                        title = host.name,
                        detail = if (host.hostId in onlineIds) {
                            stringResource(R.string.home_on_network)
                        } else {
                            "${host.address}:${host.port}"
                        },
                    ) {
                        TextButton(onClick = { onForget(host) }) { Text(stringResource(R.string.forget)) }
                        Button(onClick = { onConnect(host) }) { Text(stringResource(R.string.connect_button)) }
                    }
                }
            }

            if (unpairedFound.isNotEmpty()) {
                Section(stringResource(R.string.home_found))
                unpairedFound.forEach { host ->
                    HostRow(title = host.name, detail = "${host.address}:${host.port}") {
                        Button(onClick = { onPairFound(host) }) { Text(stringResource(R.string.pair_button)) }
                    }
                }
            }

            Spacer(Modifier.height(24.dp))
            onScan?.let {
                Button(onClick = it, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.scan_button)) }
                Spacer(Modifier.height(8.dp))
            }
            OutlinedButton(onClick = onPairManually, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.pair_with_code))
            }
            if (paired.isEmpty() && unpairedFound.isEmpty()) {
                Spacer(Modifier.height(12.dp))
                Text(stringResource(R.string.home_empty), color = Muted)
            }
            Spacer(Modifier.height(24.dp))
            TextButton(onClick = onLicenses, modifier = Modifier.align(Alignment.CenterHorizontally)) {
                Text(stringResource(R.string.licenses_button), color = Muted)
            }
        }
    }
}

@Composable
private fun Section(title: String) {
    Spacer(Modifier.height(28.dp))
    Text(title, style = MaterialTheme.typography.titleMedium)
    Spacer(Modifier.height(8.dp))
}

@Composable
private fun HostRow(title: String, detail: String, actions: @Composable () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(SurfaceColor)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleSmall)
            Text(detail, color = Muted, style = MaterialTheme.typography.bodySmall)
        }
        actions()
    }
}
