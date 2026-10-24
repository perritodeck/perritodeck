package dev.perritodeck.client.ui

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
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.perritodeck.client.PairState
import dev.perritodeck.client.R
import dev.perritodeck.client.net.Fingerprint

@Composable
fun PairScreen(
    state: PairState,
    onSubmit: (address: String, code: String) -> Unit,
    onConfirm: () -> Unit,
    onCancel: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxSize()
            // Clear of the keyboard and of the status and navigation bars, which Android 15
            // draws over the app.
            .windowInsetsPadding(WindowInsets.safeDrawing)
            .verticalScroll(rememberScrollState())
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Column(Modifier.widthIn(max = 480.dp).fillMaxWidth()) {
            when (state) {
                is PairState.Form -> PairForm(state, onSubmit, onCancel)
                is PairState.Working -> {
                    CircularProgressIndicator(color = Brand, modifier = Modifier.align(Alignment.CenterHorizontally))
                    Spacer(Modifier.height(16.dp))
                    Text(state.message, modifier = Modifier.align(Alignment.CenterHorizontally))
                    Spacer(Modifier.height(16.dp))
                    TextButton(onClick = onCancel, modifier = Modifier.align(Alignment.CenterHorizontally)) {
                        Text(stringResource(R.string.cancel))
                    }
                }
                is PairState.Confirm -> {
                    Text(stringResource(R.string.pair_confirm_title), style = MaterialTheme.typography.headlineSmall)
                    Spacer(Modifier.height(8.dp))
                    Text(stringResource(R.string.pair_confirm_body), color = Muted)
                    Spacer(Modifier.height(24.dp))
                    Text(
                        Fingerprint.display(state.fingerprint),
                        fontFamily = FontFamily.Monospace,
                        fontSize = 34.sp,
                        color = Brand,
                        modifier = Modifier.align(Alignment.CenterHorizontally).testTag("fingerprint"),
                    )
                    Spacer(Modifier.height(24.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedButton(onClick = onCancel, modifier = Modifier.weight(1f)) {
                            Text(stringResource(R.string.pair_mismatch))
                        }
                        Button(onClick = onConfirm, modifier = Modifier.weight(1f)) {
                            Text(stringResource(R.string.pair_matches))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PairForm(state: PairState.Form, onSubmit: (String, String) -> Unit, onCancel: () -> Unit) {
    var address by rememberSaveable(state) { mutableStateOf(state.address) }
    var code by rememberSaveable { mutableStateOf("") }
    val submit = { onSubmit(address, code) }
    val codeFocus = remember { FocusRequester() }

    Text(stringResource(R.string.pair_title), style = MaterialTheme.typography.headlineMedium)
    Spacer(Modifier.height(8.dp))
    Text(stringResource(R.string.pair_subtitle), color = Muted)
    Spacer(Modifier.height(24.dp))

    OutlinedTextField(
        value = address,
        onValueChange = { address = it },
        label = { Text(stringResource(R.string.host_label)) },
        placeholder = { Text("192.168.1.20") },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
        // Straight to the code: plain "next" stops on the preset buttons in between whenever
        // they can take focus (out of touch mode, e.g. typed on a keyboard), closing the keyboard.
        keyboardActions = KeyboardActions(onNext = { codeFocus.requestFocus() }),
        modifier = Modifier.fillMaxWidth().testTag("address"),
    )
    Spacer(Modifier.height(8.dp))
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        OutlinedButton(onClick = { address = "10.0.2.2" }) { Text(stringResource(R.string.preset_emulator)) }
        OutlinedButton(onClick = { address = "127.0.0.1" }) { Text(stringResource(R.string.preset_usb)) }
    }
    Spacer(Modifier.height(8.dp))
    OutlinedTextField(
        value = code,
        onValueChange = { v -> code = v.filter(Char::isDigit).take(6) },
        label = { Text(stringResource(R.string.code_label)) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Go),
        keyboardActions = KeyboardActions(onGo = { submit() }),
        modifier = Modifier.fillMaxWidth().focusRequester(codeFocus).testTag("code"),
    )
    state.error?.let {
        Spacer(Modifier.height(12.dp))
        Text(it, color = MaterialTheme.colorScheme.error)
    }
    Spacer(Modifier.height(20.dp))
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton(onClick = onCancel) { Text(stringResource(R.string.cancel)) }
        Spacer(Modifier.weight(1f))
        Button(onClick = submit) { Text(stringResource(R.string.continue_button)) }
    }
}
