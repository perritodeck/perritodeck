package dev.perritodeck.client.scan

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.perritodeck.client.R
import dev.perritodeck.client.ui.Brand
import dev.perritodeck.client.ui.Muted
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** True if this device has any camera (most Fire tablets have only a front one, which works). */
fun hasCamera(context: Context): Boolean =
    context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)

private fun hasPermission(context: Context) =
    Build.VERSION.SDK_INT < 23 ||
        ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

/**
 * Full-screen camera that looks for the host's pairing QR code. Calls [onLink] once
 * with the first `perritodeck://pair` link it sees.
 */
@Composable
fun ScanScreen(onLink: (String) -> Unit, onCancel: () -> Unit) {
    val context = LocalContext.current
    var granted by remember { mutableStateOf(hasPermission(context)) }
    var denied by remember { mutableStateOf(false) }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        granted = ok
        denied = !ok
    }
    LaunchedEffect(Unit) { if (!granted) ask.launch(Manifest.permission.CAMERA) }

    Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
        if (granted) {
            CameraPreview(onLink)
            // Aiming square.
            Box(
                Modifier
                    .size(260.dp)
                    .border(3.dp, Brand, RoundedCornerShape(24.dp))
            )
        } else if (denied) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(32.dp)) {
                Text(stringResource(R.string.scan_permission_denied), textAlign = TextAlign.Center)
                Spacer(Modifier.height(16.dp))
                Button(onClick = { ask.launch(Manifest.permission.CAMERA) }) { Text(stringResource(R.string.scan_allow)) }
            }
        }
        Column(
            Modifier.align(Alignment.BottomCenter).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(stringResource(R.string.scan_hint), color = Muted, textAlign = TextAlign.Center)
            TextButton(onClick = onCancel) { Text(stringResource(R.string.cancel)) }
        }
    }
}

@Composable
private fun CameraPreview(onLink: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val currentOnLink by rememberUpdatedState(onLink)
    val previewView = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
    val analysisExecutor = remember { Executors.newSingleThreadExecutor() }
    val done = remember { AtomicBoolean(false) }

    DisposableEffect(lifecycleOwner) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        val decoder = QrDecoder()
        providerFuture.addListener({
            val provider = providerFuture.get()
            val preview = Preview.Builder().build().also { it.setSurfaceProvider(previewView.surfaceProvider) }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
            analysis.setAnalyzer(analysisExecutor) { image ->
                image.use {
                    if (done.get()) return@use
                    val plane = it.planes[0]
                    val buffer = plane.buffer
                    val bytes = ByteArray(buffer.remaining()).also(buffer::get)
                    val text = decoder.decode(bytes, it.width, it.height, plane.rowStride)
                    if (text != null && text.startsWith("perritodeck://pair") && done.compareAndSet(false, true)) {
                        ContextCompat.getMainExecutor(context).execute { currentOnLink(text) }
                    }
                }
            }
            // Fire tablets often have only a front camera, so use whichever exists, back first.
            val selector = when {
                provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA) -> CameraSelector.DEFAULT_BACK_CAMERA
                else -> CameraSelector.DEFAULT_FRONT_CAMERA
            }
            runCatching {
                provider.unbindAll()
                provider.bindToLifecycle(lifecycleOwner, selector, preview, analysis)
            }
        }, ContextCompat.getMainExecutor(context))

        onDispose {
            runCatching { providerFuture.get().unbindAll() }
            analysisExecutor.shutdown()
        }
    }

    AndroidView(factory = { previewView }, modifier = Modifier.fillMaxSize())
}
