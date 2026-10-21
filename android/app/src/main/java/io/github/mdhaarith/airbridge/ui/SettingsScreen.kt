package io.github.mdhaarith.airbridge.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.BuildConfig
import io.github.mdhaarith.airbridge.core.Discovery
import io.github.mdhaarith.airbridge.service.AirBridgeService

@Composable
fun SettingsScreen(vm: AppViewModel) {
    val air = LocalAir.current
    val context = LocalContext.current
    val engine = vm.engine
    val settings by engine.settings.state.collectAsStateWithLifecycle()
    val running by engine.running.collectAsStateWithLifecycle()
    val peers by engine.peers.collectAsStateWithLifecycle()

    Column(
        Modifier.fillMaxSize().statusBarsPadding().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).navigationBarsPadding().padding(bottom = 24.dp),
    ) {
        Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            IconCircleButton(Icons.AutoMirrored.Rounded.ArrowBack, "Back") { vm.screen = Screen.HOME }
            Spacer(Modifier.width(12.dp))
            Text("Settings", color = air.text, fontWeight = FontWeight.Bold, fontSize = 22.sp)
        }

        SectionTitle("This phone")
        GlassCard(Modifier.fillMaxWidth()) {
            var name by remember(settings.alias) { mutableStateOf(settings.alias) }
            OutlinedTextField(
                name, { name = it.take(64) },
                label = { Text("Device name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { engine.settings.update { it.copy(alias = name) } }),
                trailingIcon = {
                    if (name != settings.alias) TextButton({ engine.settings.update { it.copy(alias = name) } }) { Text("Save") }
                },
            )
            Spacer(Modifier.height(6.dp))
            ToggleRow("Visible to nearby devices", "Receive files and show up on your computer", running) { on ->
                if (on) AirBridgeService.start(context) else AirBridgeService.stop(context)
            }
        }

        SectionTitle("Receiving")
        GlassCard(Modifier.fillMaxWidth()) {
            ToggleRow("Auto-accept trusted devices", "No prompt for devices you trust", settings.autoAcceptTrusted) { on ->
                engine.settings.update { it.copy(autoAcceptTrusted = on) }
            }
            HorizontalDivider(color = air.border)
            ToggleRow("Copy received text", "Text from trusted devices goes to your clipboard", settings.copyReceivedText) { on ->
                engine.settings.update { it.copy(copyReceivedText = on) }
            }
            HorizontalDivider(color = air.border)
            var pin by remember(settings.pin) { mutableStateOf(settings.pin) }
            OutlinedTextField(
                pin, { pin = it.filter(Char::isLetterOrDigit).take(32) },
                label = { Text("Require a PIN (optional)") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().padding(top = 10.dp),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { engine.settings.update { it.copy(pin = pin) } }),
                trailingIcon = { if (pin != settings.pin) TextButton({ engine.settings.update { it.copy(pin = pin) } }) { Text("Save") } },
            )
        }

        if (settings.trusted.isNotEmpty()) {
            SectionTitle("Trusted devices")
            GlassCard(Modifier.fillMaxWidth()) {
                settings.trusted.forEachIndexed { i, id ->
                    if (i > 0) HorizontalDivider(color = air.border)
                    val peer = peers.firstOrNull { it.id == id }
                    Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        Avatar(id, deviceIcon(peer?.deviceType, peer?.deviceModel), 34.dp)
                        Spacer(Modifier.width(12.dp))
                        Text(peer?.alias ?: "Device ${id.take(8)}…", color = air.text, modifier = Modifier.weight(1f))
                        TextButton({ engine.setTrusted(id, false) }) { Text("Remove") }
                    }
                }
            }
        }

        SectionTitle("About")
        GlassCard(Modifier.fillMaxWidth()) {
            val addresses = remember(running) { Discovery.localAddresses().joinToString { it.address.hostAddress ?: "" } }
            Text("AirBridge ${BuildConfig.VERSION_NAME}", color = air.text, fontWeight = FontWeight.SemiBold)
            Text("LocalSend protocol v2 · port ${engine.port} · ${addresses.ifEmpty { "offline" }}", color = air.muted, fontSize = 13.sp)
            Spacer(Modifier.height(8.dp))
            Text("Fingerprint", color = air.faint, fontSize = 12.sp)
            Text(engine.identity.fingerprint.chunked(4).joinToString(" "), color = air.muted, fontSize = 11.sp, fontFamily = FontFamily.Monospace)
            Spacer(Modifier.height(8.dp))
            Text("Works with AirBridge for Linux and any LocalSend app on the same Wi‑Fi.", color = air.muted, fontSize = 13.sp)
        }
    }
}

@Composable
private fun ToggleRow(title: String, subtitle: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    val air = LocalAir.current
    Row(Modifier.fillMaxWidth().padding(vertical = 8.dp).pressable({ onChange(!checked) }), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, color = air.text, fontWeight = FontWeight.SemiBold)
            Text(subtitle, color = air.muted, fontSize = 13.sp)
        }
        Switch(checked, onChange, colors = SwitchDefaults.colors(checkedTrackColor = Violet))
    }
}
