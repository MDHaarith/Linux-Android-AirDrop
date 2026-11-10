@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.mdhaarith.airbridge.ui

import android.Manifest
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Smartphone
import androidx.compose.material3.*
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import io.github.mdhaarith.airbridge.core.DeviceInfo
import io.github.mdhaarith.airbridge.core.PairCode
import io.github.mdhaarith.airbridge.core.decodeQr
import java.util.concurrent.Executors
import kotlinx.coroutines.launch

@Composable
fun ScanScreen(vm: AppViewModel) {
    val context = LocalContext.current
    val colors = MaterialTheme.colorScheme
    val scope = rememberCoroutineScope()
    fun granted(permission: String) =
        ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED
    var cameraAllowed by remember { mutableStateOf(granted(Manifest.permission.CAMERA)) }
    var code by remember { mutableStateOf<PairCode?>(null) }
    var waiting by remember { mutableStateOf<PairCode?>(null) }
    var device by remember { mutableStateOf<DeviceInfo?>(null) }
    var message by remember { mutableStateOf<String?>(null) }
    // The camera keeps seeing a code after it failed or was cancelled; don't reconnect by itself.
    var dismissed by remember { mutableStateOf<PairCode?>(null) }
    var pairing by remember { mutableStateOf(false) }
    val camera =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
            cameraAllowed = it
        }
    val nearby =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
            if (allowed) code = waiting
            else message = "Joining the other phone's network needs the Nearby Wi-Fi permission."
            waiting = null
        }
    LaunchedEffect(Unit) { if (!cameraAllowed) camera.launch(Manifest.permission.CAMERA) }
    LaunchedEffect(code) {
        val scanned = code ?: return@LaunchedEffect
        vm.reachPair(context, scanned)
            .onSuccess { device = it }
            .onFailure {
                message = it.message
                dismissed = scanned
                code = null
            }
    }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Pair device") },
                navigationIcon = {
                    IconButton(onClick = { vm.screen = Screen.HOME }) {
                        Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back")
                    }
                },
                actions = {},
            )
        }
    ) { insets ->
        Box(Modifier.fillMaxSize().padding(insets)) {
            Column(
                Modifier.verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp)
                    .padding(top = 12.dp, bottom = 24.dp)
            ) {
                val found = device
                val scanned = code
                when {
                    found != null && scanned != null ->
                        PairConfirm(
                            found,
                            onPair = {
                                if (pairing) return@PairConfirm
                                pairing = true
                                scope.launch {
                                    if (vm.confirmPair(scanned, found)) vm.screen = Screen.HOME
                                    else {
                                        message = "This QR code has expired. Show a new one."
                                        dismissed = scanned
                                        device = null
                                        code = null
                                    }
                                    pairing = false
                                }
                            },
                            onCancel = {
                                vm.leaveDirectNetwork()
                                dismissed = scanned
                                device = null
                                code = null
                            },
                        )
                    scanned != null ->
                        Column(
                            Modifier.fillMaxWidth().padding(vertical = 48.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            CircularProgressIndicator(
                                color = colors.primary,
                                strokeWidth = 3.dp,
                                modifier = Modifier.size(32.dp),
                            )
                            Spacer(Modifier.height(16.dp))
                            Text(
                                "Connecting…",
                                color = colors.onSurfaceVariant,
                                style = MaterialTheme.typography.bodyLarge,
                            )
                        }
                    !cameraAllowed ->
                        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                            ListItem(
                                headlineContent = {
                                    Text(
                                        "Camera access is off",
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                },
                                supportingContent = {
                                    Text("AirBridge needs the camera to scan the code")
                                },
                                leadingContent = {
                                    Icon(Icons.Rounded.QrCodeScanner, null, tint = colors.tertiary)
                                },
                                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            )

                            Text(
                                "Allow the camera for AirBridge in the phone's settings, then come back.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                            )
                        }
                    else -> {
                        QrCamera(
                            Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(12.dp))
                        ) { text ->
                            val parsed = PairCode.parse(text)
                            when {
                                parsed == null -> message = "This isn't an AirBridge code."
                                waiting != null || parsed == dismissed -> Unit
                                parsed.ssid != null && !granted(wifiDirectPermission) -> {
                                    waiting = parsed
                                    nearby.launch(wifiDirectPermission)
                                }
                                else -> {
                                    message = null
                                    code = parsed
                                }
                            }
                        }
                        Spacer(Modifier.height(22.dp))
                        ScanHints()
                    }
                }
                message?.let {
                    Text(
                        it,
                        color = colors.error,
                        style = MaterialTheme.typography.bodyLarge,
                        modifier = Modifier.padding(horizontal = 16.dp),
                    )
                }
            }
        }
    }
}

/** Where the code to scan is shown. */
@Composable
internal fun ScanHints() {
    Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
        Text(
            "Where to find the code",
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )

        ListItem(
            headlineContent = {
                Text("On your computer", color = MaterialTheme.colorScheme.onSurface)
            },
            supportingContent = { Text("Open AirBridge and click the QR button") },
            leadingContent = {
                Icon(
                    Icons.Rounded.Computer,
                    null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )
        HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
        ListItem(
            headlineContent = {
                Text("On another phone", color = MaterialTheme.colorScheme.onSurface)
            },
            supportingContent = { Text("Share to Any Phone, then Other phone has AirBridge") },
            leadingContent = {
                Icon(
                    Icons.Rounded.Smartphone,
                    null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        )

        Text(
            "Pair once. After that your devices find each other on the same Wi‑Fi, or anywhere over Tailscale.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
    }
}

/** The scanned device, confirmed by name before anything is trusted. */
@Composable
internal fun PairConfirm(device: DeviceInfo, onPair: () -> Unit, onCancel: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Column(
        Modifier.fillMaxWidth().padding(top = 28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Avatar(deviceIcon(device.deviceType, device.deviceModel), 88.dp)
        Spacer(Modifier.height(16.dp))
        Text(
            "Pair with ${device.alias}?",
            color = colors.onSurface,
            style = MaterialTheme.typography.headlineSmall,
            textAlign = TextAlign.Center,
        )
        device.deviceModel?.let {
            Text(it, color = colors.onSurfaceVariant, style = MaterialTheme.typography.bodyLarge)
        }
        Spacer(Modifier.height(16.dp))
        Text(
            "You'll trust each other: what this device sends is accepted without asking. Only pair with your own devices.",
            color = colors.onSurfaceVariant,
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
        Spacer(Modifier.height(24.dp))
        Button(onClick = onPair, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
            Text("Pair")
        }
        Spacer(Modifier.height(8.dp))
        FilledTonalButton(
            onClick = onCancel,
            modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
        ) {
            Text("Cancel")
        }
    }
}

@Composable
private fun QrCamera(modifier: Modifier, onText: (String) -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val latest by rememberUpdatedState(onText)
    val view = remember { PreviewView(context) }
    DisposableEffect(lifecycle) {
        val executor = Executors.newSingleThreadExecutor()
        val future = ProcessCameraProvider.getInstance(context)
        val main = ContextCompat.getMainExecutor(context)
        var disposed = false
        future.addListener(
            {
                if (disposed) return@addListener
                val preview =
                    Preview.Builder().build().also { it.surfaceProvider = view.surfaceProvider }
                val analysis =
                    ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build()
                analysis.setAnalyzer(executor) { image ->
                    image
                        .use {
                            val plane = it.planes[0]
                            val bytes = ByteArray(plane.buffer.remaining()).also(plane.buffer::get)
                            decodeQr(bytes, plane.rowStride, it.width, it.height)
                        }
                        ?.let { text -> main.execute { latest(text) } }
                }
                runCatching {
                    future
                        .get()
                        .bindToLifecycle(
                            lifecycle,
                            CameraSelector.DEFAULT_BACK_CAMERA,
                            preview,
                            analysis,
                        )
                }
            },
            main,
        )
        onDispose {
            disposed = true
            future.addListener({ runCatching { future.get().unbindAll() } }, main)
            executor.shutdown()
        }
    }
    AndroidView({ view }, modifier)
}
