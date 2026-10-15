package dev.perritodeck.client.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import dev.perritodeck.client.BuildConfig
import dev.perritodeck.client.R
import dev.perritodeck.client.data.ThirdPartyLicenses
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The licence list once it's been read; [document] is null when the asset is missing. */
private class Loaded(val document: ThirdPartyLicenses.Document?)

/**
 * Perrito's own licence, a link to its source, and the libraries it includes with their
 * licence texts (the asset the build generates; see data/ThirdPartyLicenses.kt).
 */
@Composable
fun LicensesScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val loaded by produceState<Loaded?>(initialValue = null) {
        value = withContext(Dispatchers.IO) {
            val text = runCatching {
                context.assets.open(ThirdPartyLicenses.ASSET).bufferedReader(Charsets.UTF_8).use { it.readText() }
            }.getOrNull()
            Loaded(text?.let(ThirdPartyLicenses::parse))
        }
    }

    LazyColumn(
        // Clear of the status and navigation bars, which Android 15 draws over the app.
        Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing),
        contentPadding = PaddingValues(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        block {
            TextButton(onClick = onBack, modifier = Modifier.padding(bottom = 8.dp)) { Text(stringResource(R.string.back)) }
            Text(stringResource(R.string.licenses_title), style = MaterialTheme.typography.headlineSmall)
            Spacer(Modifier.height(12.dp))
            Text(stringResource(R.string.licenses_intro, BuildConfig.VERSION_NAME))
            Spacer(Modifier.height(12.dp))
            Text(stringResource(R.string.licenses_source), style = MaterialTheme.typography.titleSmall)
            TextButton(onClick = { openLink(context, BuildConfig.SOURCE_URL) }) {
                Text(BuildConfig.SOURCE_URL.removePrefix("https://"))
            }
        }

        val document = loaded?.document
        when {
            loaded == null -> block {
                CircularProgressIndicator(color = Brand, modifier = Modifier.align(Alignment.CenterHorizontally).padding(24.dp))
            }
            document == null -> block {
                Spacer(Modifier.height(16.dp))
                Text(stringResource(R.string.licenses_missing), color = MaterialTheme.colorScheme.error)
            }
            else -> licenses(document)
        }
    }
}

private fun LazyListScope.licenses(document: ThirdPartyLicenses.Document) {
    block {
        Spacer(Modifier.height(16.dp))
        Text(stringResource(R.string.licenses_libraries), color = Muted)
        Spacer(Modifier.height(8.dp))
    }
    document.libraries.forEachIndexed { i, library ->
        block(key = "library-$i") {
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 3.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(SurfaceColor)
                    .padding(horizontal = 14.dp, vertical = 10.dp),
            ) {
                Text(library.title, style = MaterialTheme.typography.titleSmall)
                library.details.forEach { Text(it, color = Muted, style = MaterialTheme.typography.bodySmall) }
            }
        }
    }
    document.texts.forEachIndexed { s, section ->
        block(key = "title-$s") {
            Spacer(Modifier.height(28.dp))
            Text(section.title, style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(4.dp))
        }
        section.paragraphs.forEachIndexed { p, lines ->
            val paragraph = ThirdPartyLicenses.reflow(lines)
            block(key = "text-$s-$p") {
                Text(
                    paragraph.text,
                    style = if (paragraph.heading) MaterialTheme.typography.titleSmall else MaterialTheme.typography.bodySmall,
                    textAlign = if (paragraph.centered) TextAlign.Center else TextAlign.Start,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = (paragraph.indent.coerceAtMost(12) * 3).dp, top = 6.dp),
                )
            }
        }
    }
}

/** One item of the list: a column at most 640 dp wide, centred, so lines stay readable on a tablet. */
private fun LazyListScope.block(key: Any? = null, content: @Composable ColumnScope.() -> Unit) {
    item(key = key) {
        Column(Modifier.widthIn(max = 640.dp).fillMaxWidth(), content = content)
    }
}

private fun openLink(context: Context, url: String) {
    try {
        context.startActivity(Intent(Intent.ACTION_VIEW, url.toUri()))
    } catch (_: ActivityNotFoundException) {
        Toast.makeText(context, R.string.licenses_no_browser, Toast.LENGTH_LONG).show()
    }
}
