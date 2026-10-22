package io.github.mdhaarith.airbridge.ui

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.OpenInNew
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material.icons.rounded.VerticalSplit
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.BuildConfig
import io.github.mdhaarith.airbridge.core.Discovery
import io.github.mdhaarith.airbridge.overlay.EdgePanel
import io.github.mdhaarith.airbridge.service.AirBridgeService

@Composable
fun SettingsScreen(vm: AppViewModel) {
    val air = LocalAir.current
    val context = LocalContext.current
    val engine = vm.engine
    val settings by engine.settings.state.collectAsStateWithLifecycle()
    val running by engine.running.collectAsStateWithLifecycle()
    val peers by engine.peers.collectAsStateWithLifecycle()
    var renaming by remember { mutableStateOf(false) }
    var pinning by remember { mutableStateOf(false) }

    val photoPermissions = remember {
        when {
            Build.VERSION.SDK_INT >= 34 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
            Build.VERSION.SDK_INT >= 33 -> arrayOf(Manifest.permission.READ_MEDIA_IMAGES)
            else -> arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
    }
    val askPhotos = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { }

    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        NavBar("Settings", back = "AirBridge", onBack = { vm.screen = Screen.HOME })
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).navigationBarsPadding().padding(top = 12.dp, bottom = 24.dp)) {
            Group(footer = "Your computer and other devices on this Wi‑Fi see this phone while it is visible.") {
                ListRow("Name", onClick = { renaming = true }) {
                    Text(settings.alias, color = air.muted, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 180.dp))
                }
                RowSeparator()
                ListRow("Visible to Nearby Devices", chevron = false) {
                    AppleSwitch(running, { on -> if (on) AirBridgeService.start(context) else AirBridgeService.stop(context) })
                }
            }

            Group("Receiving", footer = "Trust your own devices from their menu on the home screen. A PIN keeps unknown senders out.") {
                ListRow("Accept From My Devices", subtitle = "No prompt for devices you trust", icon = Icons.Rounded.Wifi, chevron = false) {
                    AppleSwitch(settings.autoAcceptTrusted, { on -> engine.settings.update { it.copy(autoAcceptTrusted = on) } })
                }
                RowSeparator(59.dp)
                ListRow("Open What They Send", subtitle = "A photo or link from your own devices opens right away", icon = Icons.Rounded.OpenInNew, chevron = false) {
                    AppleSwitch(settings.openReceived, { on -> engine.settings.update { it.copy(openReceived = on) } })
                }
                RowSeparator(59.dp)
                ListRow("Copy Received Text", subtitle = "Text from your devices goes to the clipboard", icon = Icons.Rounded.ContentCopy, chevron = false) {
                    AppleSwitch(settings.copyReceivedText, { on -> engine.settings.update { it.copy(copyReceivedText = on) } })
                }
                RowSeparator(59.dp)
                ListRow("Require a PIN", icon = Icons.Rounded.Lock, iconColor = air.muted, onClick = { pinning = true }) {
                    Text(if (settings.pin.isEmpty()) "Off" else "On", color = air.muted, fontSize = 17.sp)
                }
            }

            Group("Sending", footer = "Your devices also appear in Android's share sheet: share anything and tap your computer.") {
                ListRow("Edge Panel", subtitle = "A handle on the screen edge: send your clipboard, latest photo or anything you drop on it", icon = Icons.Rounded.VerticalSplit, chevron = false) {
                    AppleSwitch(settings.edgePanel, { on ->
                        engine.settings.update { it.copy(edgePanel = on) }
                        if (on) {
                            askPhotos.launch(photoPermissions)
                            if (!android.provider.Settings.canDrawOverlays(context)) context.startActivity(EdgePanel.permissionIntent(context))
                            if (!running) AirBridgeService.start(context)
                        }
                    })
                }
                if (settings.edgePanel) {
                    RowSeparator(59.dp)
                    ListRow("Handle on the Left", icon = Icons.Rounded.SwapHoriz, iconColor = air.muted, chevron = false) {
                        AppleSwitch(settings.edgeSide == "left", { left -> engine.settings.update { it.copy(edgeSide = if (left) "left" else "right") } })
                    }
                }
            }

            if (settings.trusted.isNotEmpty()) {
                Group("My Devices") {
                    settings.trusted.forEachIndexed { i, id ->
                        if (i > 0) RowSeparator(66.dp)
                        val peer = peers.firstOrNull { it.id == id }
                        ListRow(peer?.alias ?: "Device ${id.take(8)}…", subtitle = peer?.deviceModel, chevron = false) {
                            Text("Remove", color = air.danger, fontSize = 15.sp, modifier = Modifier.pressable({ engine.setTrusted(id, false) }))
                        }
                    }
                }
            }

            Group("About") {
                val addresses = remember(running) { Discovery.localAddresses().joinToString { it.address.hostAddress ?: "" } }
                ListRow("Version", chevron = false) { Text(BuildConfig.VERSION_NAME, color = air.muted, fontSize = 17.sp) }
                RowSeparator()
                ListRow("Address", chevron = false) { Text("${addresses.ifEmpty { "offline" }} · ${engine.port}", color = air.muted, fontSize = 15.sp) }
                RowSeparator()
                Column(Modifier.padding(horizontal = 16.dp, vertical = 11.dp)) {
                    Text("Fingerprint", color = air.text, fontSize = 17.sp)
                    Spacer(Modifier.height(2.dp))
                    Text(engine.identity.fingerprint.chunked(4).joinToString(" "), color = air.muted, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
                }
            }
            Text(
                "Works with AirBridge for Linux, Quick Share on the computer side, and any LocalSend app on the same Wi‑Fi.",
                color = air.muted, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
    }

    if (renaming) {
        TextAlert(
            title = "Device Name",
            message = "This is how your computer sees this phone.",
            initial = settings.alias,
            confirm = "Save",
            filter = { it.take(64) },
            onConfirm = { name -> renaming = false; engine.settings.update { it.copy(alias = name) } },
            onDismiss = { renaming = false },
        )
    }
    if (pinning) {
        TextAlert(
            title = "Require a PIN",
            message = "Senders must enter it before files arrive. Leave it empty for no PIN.",
            initial = settings.pin,
            confirm = "Save",
            keyboardType = KeyboardType.Number,
            allowEmpty = true,
            filter = { it.filter(Char::isLetterOrDigit).take(32) },
            onConfirm = { pin -> pinning = false; engine.settings.update { it.copy(pin = pin) } },
            onDismiss = { pinning = false },
        )
    }
}
