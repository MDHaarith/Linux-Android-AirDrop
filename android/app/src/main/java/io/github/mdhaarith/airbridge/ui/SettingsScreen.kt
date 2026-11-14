@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.mdhaarith.airbridge.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ScrollState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.OpenInNew
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.SwapHoriz
import androidx.compose.material.icons.rounded.VerticalSplit
import androidx.compose.material.icons.rounded.Badge
import androidx.compose.material.icons.rounded.VerifiedUser
import androidx.compose.material.icons.rounded.Visibility
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.BuildConfig
import io.github.mdhaarith.airbridge.overlay.EdgePanel
import io.github.mdhaarith.airbridge.photoPermissions
import io.github.mdhaarith.airbridge.service.AirBridgeService

@Composable
fun SettingsScreen(vm: AppViewModel, scrollState: ScrollState = rememberScrollState()) {
    val colors = MaterialTheme.colorScheme
    val context = LocalContext.current
    val deviceAddresses = LocalDeviceAddresses.current
    val engine = vm.engine
    val settings by engine.settings.state.collectAsStateWithLifecycle()
    val running by engine.running.collectAsStateWithLifecycle()
    val peers by engine.peers.collectAsStateWithLifecycle()
    var renaming by remember { mutableStateOf(false) }
    var pinning by remember { mutableStateOf(false) }

    val askPhotos =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {}

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
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
            Column(Modifier.verticalScroll(scrollState).padding(top = 12.dp, bottom = 24.dp)) {
                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    ListItem(
                        headlineContent = {
                            Text("Name", color = MaterialTheme.colorScheme.onSurface)
                        },
                        leadingContent = {
                            Icon(Icons.Rounded.Badge, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    settings.alias,
                                    color = colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.titleMedium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.widthIn(max = 180.dp),
                                )
                            }
                        },
                        modifier = Modifier.clickable(onClick = { renaming = true }),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text(
                                "Visible to nearby devices",
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        },
                        leadingContent = {
                            Icon(Icons.Rounded.Visibility, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Switch(
                                    modifier =
                                        Modifier.semantics {
                                            contentDescription = "Visible to nearby devices"
                                        },
                                    checked = running,
                                    onCheckedChange = { on ->
                                        if (on) AirBridgeService.start(context)
                                        else AirBridgeService.stop(context)
                                    },
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )

                    Text(
                        "Your computer and other devices on this Wi‑Fi see this phone while it is visible.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }

                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    Text(
                        "Receiving",
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )

                    ListItem(
                        headlineContent = {
                            Text(
                                "Accept from my devices",
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        },
                        supportingContent = { Text("No prompt for devices you trust") },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.VerifiedUser,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Switch(
                                    modifier =
                                        Modifier.semantics {
                                            contentDescription = "Accept from my devices"
                                        },
                                    checked = settings.autoAcceptTrusted,
                                    onCheckedChange = { on ->
                                        engine.settings.update { it.copy(autoAcceptTrusted = on) }
                                    },
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text("Open what they send", color = MaterialTheme.colorScheme.onSurface)
                        },
                        supportingContent = {
                            Text("A photo or link from your own devices opens right away")
                        },
                        leadingContent = {
                            Icon(
                                Icons.AutoMirrored.Rounded.OpenInNew,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Switch(
                                    modifier =
                                        Modifier.semantics {
                                            contentDescription = "Open what they send"
                                        },
                                    checked = settings.openReceived,
                                    onCheckedChange = { on ->
                                        engine.settings.update { it.copy(openReceived = on) }
                                    },
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text("Copy received text", color = MaterialTheme.colorScheme.onSurface)
                        },
                        supportingContent = {
                            Text("Text from your devices goes to the clipboard")
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.ContentCopy,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Switch(
                                    modifier =
                                        Modifier.semantics {
                                            contentDescription = "Copy received text"
                                        },
                                    checked = settings.copyReceivedText,
                                    onCheckedChange = { on ->
                                        engine.settings.update { it.copy(copyReceivedText = on) }
                                    },
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text("Require a PIN", color = MaterialTheme.colorScheme.onSurface)
                        },
                        leadingContent = {
                            Icon(Icons.Rounded.Lock, null, tint = colors.onSurfaceVariant)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    if (settings.pin.isEmpty()) "Off" else "On",
                                    color = colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.titleMedium,
                                )
                            }
                        },
                        modifier = Modifier.clickable(onClick = { pinning = true }),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )

                    Text(
                        "Trust your own devices from their menu on the home screen. A PIN keeps unknown senders out.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }

                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    Text(
                        "Sending",
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )

                    ListItem(
                        headlineContent = {
                            Text("Edge panel", color = MaterialTheme.colorScheme.onSurface)
                        },
                        supportingContent = {
                            Text(
                                "A handle on the screen edge: send your clipboard, latest photo or anything you drop on it"
                            )
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.VerticalSplit,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Switch(
                                    modifier =
                                        Modifier.semantics { contentDescription = "Edge panel" },
                                    checked = settings.edgePanel,
                                    onCheckedChange = { on ->
                                        engine.settings.update { it.copy(edgePanel = on) }
                                        if (on) {
                                            askPhotos.launch(photoPermissions)
                                            if (!android.provider.Settings.canDrawOverlays(context))
                                                context.startActivity(
                                                    EdgePanel.permissionIntent(context)
                                                )
                                            if (!running) AirBridgeService.start(context)
                                        }
                                    },
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    if (settings.edgePanel) {
                        HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                        ListItem(
                            headlineContent = {
                                Text(
                                    "Handle on the left",
                                    color = MaterialTheme.colorScheme.onSurface,
                                )
                            },
                            leadingContent = {
                                Icon(Icons.Rounded.SwapHoriz, null, tint = colors.onSurfaceVariant)
                            },
                            trailingContent = {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Switch(
                                        modifier =
                                            Modifier.semantics {
                                                contentDescription = "Handle on the left"
                                            },
                                        checked = settings.edgeSide == "left",
                                        onCheckedChange = { left ->
                                            engine.settings.update {
                                                it.copy(edgeSide = if (left) "left" else "right")
                                            }
                                        },
                                    )
                                }
                            },
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        )
                        HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                        Text(
                            "Handle",
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                        val handles = listOf("bubble", "bump")
                        SingleChoiceSegmentedButtonRow(
                            Modifier.fillMaxWidth().padding(horizontal = 16.dp)
                        ) {
                            listOf("Bubble", "Bump").forEachIndexed { index, label ->
                                SegmentedButton(
                                    selected = settings.edgeHandle == handles[index],
                                    onClick = {
                                        engine.settings.update {
                                            it.copy(edgeHandle = handles[index])
                                        }
                                    },
                                    shape = SegmentedButtonDefaults.itemShape(index, handles.size),
                                ) {
                                    Text(label)
                                }
                            }
                        }
                        Text(
                            "When dragging",
                            style = MaterialTheme.typography.titleSmall,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )
                        val styles = listOf("icons", "names", "row")
                        SingleChoiceSegmentedButtonRow(
                            Modifier.fillMaxWidth().padding(horizontal = 16.dp)
                        ) {
                            listOf("Icons", "Names", "Row").forEachIndexed { index, label ->
                                SegmentedButton(
                                    selected = settings.edgeStyle == styles[index],
                                    onClick = {
                                        engine.settings.update {
                                            it.copy(edgeStyle = styles[index])
                                        }
                                    },
                                    shape = SegmentedButtonDefaults.itemShape(index, styles.size),
                                ) {
                                    Text(label)
                                }
                            }
                        }
                    }

                    Text(
                        "Your devices also appear in Android's share sheet: share anything and tap your computer." +
                            if (settings.edgePanel)
                                "\n\nWhile the handle is on, Android shows an “AirBridge is displaying over other apps” notification. " +
                                    "Long-press it to hide it; the handle keeps working."
                            else "",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }

                if (settings.trusted.isNotEmpty()) {
                    Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                        Text(
                            "My devices",
                            style = MaterialTheme.typography.titleSmall,
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                        )

                        settings.trusted.forEachIndexed { i, id ->
                            if (i > 0) HorizontalDivider(modifier = Modifier.padding(start = 72.dp))
                            val peer = peers.firstOrNull { it.id == id }
                            ListItem(
                                headlineContent = {
                                    Text(
                                        peer?.alias ?: "Device ${id.take(8)}…",
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                },
                                supportingContent = { peer?.deviceModel?.let { Text(it) } },
                                trailingContent = {
                                    Row(verticalAlignment = Alignment.CenterVertically) {
                                        TextButton(onClick = { engine.setTrusted(id, false) }) {
                                            Text("Remove", color = colors.error)
                                        }
                                    }
                                },
                                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            )
                        }
                    }
                }

                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    Text(
                        "About",
                        style = MaterialTheme.typography.titleSmall,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )

                    val addresses = remember(running) { deviceAddresses().joinToString() }
                    ListItem(
                        headlineContent = {
                            Text("Version", color = MaterialTheme.colorScheme.onSurface)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    BuildConfig.VERSION_NAME,
                                    color = colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.titleMedium,
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 16.dp))
                    ListItem(
                        headlineContent = {
                            Text("Update", color = MaterialTheme.colorScheme.onSurface)
                        },
                        supportingContent = { Text("Install the newest release from GitHub") },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    vm.updateStatus ?: "Check",
                                    color =
                                        if (vm.updateStatus == null) colors.primary
                                        else colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.titleMedium,
                                )
                            }
                        },
                        modifier = Modifier.clickable(onClick = { vm.updateApp(context) }),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 16.dp))
                    ListItem(
                        headlineContent = {
                            Text("Address", color = MaterialTheme.colorScheme.onSurface)
                        },
                        trailingContent = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    "${addresses.ifEmpty { "offline" }} · ${engine.port}",
                                    color = colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                            }
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 16.dp))
                    Column(Modifier.padding(horizontal = 16.dp, vertical = 11.dp)) {
                        Text(
                            "Fingerprint",
                            color = colors.onSurface,
                            style = MaterialTheme.typography.titleMedium,
                        )
                        Spacer(Modifier.height(2.dp))
                        Text(
                            engine.identity.fingerprint.chunked(4).joinToString(" "),
                            color = colors.onSurfaceVariant,
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                }
                Text(
                    "Works with AirBridge for Linux, Quick Share on the computer side, and any LocalSend app on the same Wi‑Fi.",
                    color = colors.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )
            }
        }
    }

    if (renaming) {
        TextAlert(
            title = "Device name",
            message = "This is how your computer sees this phone.",
            initial = settings.alias,
            confirm = "Save",
            filter = { it.take(64) },
            onConfirm = { name ->
                renaming = false
                engine.settings.update { it.copy(alias = name) }
            },
            onDismiss = { renaming = false },
        )
    }
    if (pinning) {
        TextAlert(
            title = "Require a PIN",
            message =
                "Senders must enter it (at least 6 characters) before files arrive. Leave it empty for no PIN.",
            initial = settings.pin,
            confirm = "Save",
            keyboardType = KeyboardType.Number,
            allowEmpty = true,
            filter = { it.filter(Char::isLetterOrDigit).take(32) },
            onConfirm = { pin ->
                // Short PINs fall to guessing.
                if (pin.isNotEmpty() && pin.length < 6) {
                    vm.toast(
                        "Use at least 6 characters",
                        "Or leave it empty for no PIN",
                        good = false,
                    )
                } else {
                    pinning = false
                    engine.settings.update { it.copy(pin = pin) }
                }
            },
            onDismiss = { pinning = false },
        )
    }
}
