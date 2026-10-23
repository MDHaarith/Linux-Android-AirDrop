package io.github.mdhaarith.airbridge.ui

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import io.github.mdhaarith.airbridge.core.WifiDirectCompanion
import io.github.mdhaarith.airbridge.service.AirBridgeService

@Composable
fun CompanionScreen(vm: AppViewModel) {
    val context = LocalContext.current
    val controller = remember { WifiDirectCompanion(context, vm.engine) }
    val state by controller.state.collectAsStateWithLifecycle()
    var step by remember { mutableIntStateOf(1) }
    var permissionError by remember { mutableStateOf(false) }
    val permission = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.NEARBY_WIFI_DEVICES else Manifest.permission.ACCESS_FINE_LOCATION
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) { AirBridgeService.start(context); controller.start() } else permissionError = true
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        if (controller.offer(uris.map { it.toString() }) == null && uris.isNotEmpty()) vm.toast("Couldn't offer files", good = false)
    }
    DisposableEffect(controller) {
        if (ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED) {
            AirBridgeService.start(context); controller.start()
        } else request.launch(permission)
        onDispose { controller.dispose() }
    }
    Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().verticalScroll(rememberScrollState()).padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SecondaryButton("Stop and go back") { vm.screen = Screen.HOME }
        Text(color = LocalAir.current.text, text = "Share with a phone without the app")
        Text(color = LocalAir.current.text, text = if (permissionError) "Nearby Wi-Fi permission is required. Go back and retry to grant access." else state.message)
        if (state.url.isNotEmpty()) {
            Text(color = LocalAir.current.text, text = "1. On the other phone, scan to join Wi-Fi. Stay connected even if it says no internet.")
            Text(color = LocalAir.current.text, text = "Network: ${state.ssid}\nPassword: ${state.password}")
            SecondaryButton(if (step == 1) "Joined Wi-Fi — show link QR" else "Show Wi-Fi QR") { step = if (step == 1) 2 else 1 }
            val value = if (step == 1) WifiDirectCompanion.wifiQr(state.ssid, state.password) else state.url
            val bitmap = remember(value) {
                val matrix = QRCodeWriter().encode(value, BarcodeFormat.QR_CODE, 600, 600)
                Bitmap.createBitmap(600, 600, Bitmap.Config.ARGB_8888).apply {
                    setPixels(IntArray(600 * 600) { i -> if (matrix[i % 600, i / 600]) android.graphics.Color.BLACK else android.graphics.Color.WHITE }, 0, 600, 0, 0, 600, 600)
                }.asImageBitmap()
            }
            Image(bitmap, contentDescription = if (step == 1) "Wi-Fi join QR code" else "Browser link QR code", modifier = Modifier.fillMaxWidth().aspectRatio(1f))
            if (step == 2) { Text(color = LocalAir.current.text, text = "2. Scan to open AirBridge in the browser."); Text(color = LocalAir.current.text, text = state.url) }
            PrimaryButton("Offer files to the other phone") { picker.launch(arrayOf("*/*")) }
            Text(color = LocalAir.current.text, text = "To receive files, choose Files in the other phone's browser and accept here. Session ends after 30 minutes or when this screen closes.")
        }
    }
}
