@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.mdhaarith.airbridge.ui

import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.QrCode2
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.material3.*
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.service.AirBridgeService
import kotlinx.coroutines.delay

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(vm: AppViewModel) {
    val colors = MaterialTheme.colorScheme
    val context = LocalContext.current
    val haptics = LocalHapticFeedback.current
    val engine = vm.engine
    val peers by engine.peers.collectAsStateWithLifecycle()
    val transfers by engine.transfers.collectAsStateWithLifecycle()
    val settings by engine.settings.state.collectAsStateWithLifecycle()
    val running by engine.running.collectAsStateWithLifecycle()
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(500)
            now = System.currentTimeMillis()
        }
    }
    LaunchedEffect(Unit) {
        engine.events.collect { event ->
            if (event is EngineEvent.Finished && event.transfer.status == Status.DONE)
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
        }
    }

    var menuFor by remember { mutableStateOf<Peer?>(null) }
    var composeFor by remember { mutableStateOf<Peer?>(null) }
    var composing by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var target by remember { mutableStateOf<Peer?>(null) }

    val pickFiles =
        rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
            val peer = target
            target = null
            if (uris.isEmpty()) return@rememberLauncherForActivityResult
            if (peer != null) vm.sendFiles(peer, uris) else vm.stage(uris)
        }
    val pickMedia =
        rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris
            ->
            val peer = target
            target = null
            if (uris.isEmpty()) return@rememberLauncherForActivityResult
            if (peer != null) vm.sendFiles(peer, uris) else vm.stage(uris)
        }
    fun readClipboard(): String? =
        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
            .primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(context)
            ?.toString()
            ?.takeIf { it.isNotBlank() }

    val deviceAddresses = LocalDeviceAddresses.current
    val deviceColumns = if (LocalDensity.current.fontScale > 1.3f) 2 else 3
    val activeCount = transfers.count { !it.status.isFinal }
    val pending = vm.pending

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("AirBridge", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                actions = {
                    val spin =
                        rememberInfiniteTransition(label = "scan")
                            .animateFloat(
                                0f,
                                360f,
                                infiniteRepeatable(tween(1000, easing = LinearEasing)),
                                label = "s",
                            )
                    IconButton(
                        onClick = { vm.scan() },
                        modifier =
                            Modifier.graphicsLayer {
                                rotationZ = if (vm.scanning) spin.value else 0f
                            },
                    ) {
                        Icon(Icons.Rounded.Refresh, "Look for devices")
                    }
                    IconButton(onClick = { vm.screen = Screen.ACTIVITY }, modifier = Modifier) {
                        BadgedBox(
                            badge = { if (activeCount > 0) Badge { Text("${activeCount}") } }
                        ) {
                            Icon(Icons.Rounded.Schedule, "Activity")
                        }
                    }
                    IconButton(onClick = { vm.screen = Screen.SETTINGS }, modifier = Modifier) {
                        Icon(Icons.Rounded.Settings, "Settings")
                    }
                },
            )
        }
    ) { insets ->
        Box(Modifier.fillMaxSize().padding(insets).consumeWindowInsets(insets)) {
            Column(
                Modifier.fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp)
                    .padding(top = 8.dp, bottom = 24.dp)
            ) {
                // ------------------------------------------------- this phone
                val ip = remember(running, peers.size) { deviceAddresses().firstOrNull() }
                ElevatedCard(
                    onClick = { if (running) renaming = true else AirBridgeService.start(context) },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    ListItem(
                        headlineContent = {
                            Text(if (running) "Visible as ${settings.alias}" else "Hidden")
                        },
                        supportingContent = {
                            Text(
                                if (running)
                                    listOfNotNull("Ready to receive", ip).joinToString(" · ")
                                else "Tap to become visible to nearby devices"
                            )
                        },
                        leadingContent = { Beacon(running, size = 24.dp) },
                        trailingContent = { if (running) Icon(Icons.Rounded.Edit, "Rename") },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                }

                // ---------------------------------------------- what is waiting
                AnimatedVisibility(
                    pending != null,
                    enter = slideInVertically(tween(300)) { -it / 2 } + fadeIn(tween(200)),
                    exit = slideOutVertically(tween(250)) { -it / 2 } + fadeOut(tween(200)),
                ) {
                    val share = pending ?: PendingShare()
                    Card(modifier = Modifier.fillMaxWidth().padding(top = 12.dp)) {
                        Column(Modifier.padding(12.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Box(
                                    Modifier.size(42.dp)
                                        .clip(RoundedCornerShape(9.dp))
                                        .background(colors.primary.copy(alpha = 0.12f)),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Icon(
                                        if (share.text != null) Icons.Rounded.ChatBubble
                                        else Icons.Rounded.AttachFile,
                                        null,
                                        tint = colors.primary,
                                    )
                                }
                                Spacer(Modifier.width(12.dp))
                                Column(Modifier.weight(1f)) {
                                    Text(
                                        share.label,
                                        color = colors.onSurface,
                                        fontWeight = FontWeight.SemiBold,
                                        style = MaterialTheme.typography.bodyLarge,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                    Text(
                                        (if (share.text == null)
                                            "${formatBytes(share.totalBytes)} · "
                                        else "") +
                                            if (peers.isEmpty()) "waiting for a device…"
                                            else "tap a device below",
                                        color = colors.primary,
                                        style = MaterialTheme.typography.bodySmall,
                                    )
                                }
                                IconButton(onClick = { vm.clearPending() }) {
                                    Icon(
                                        Icons.Rounded.Close,
                                        "Clear",
                                        tint = colors.onSurfaceVariant,
                                        modifier = Modifier.size(16.dp),
                                    )
                                }
                            }
                        }
                    }
                }

                // ------------------------------------------------------ devices
                SectionHeader(
                    if (pending != null) "Send to" else "Nearby",
                    trailing = if (vm.scanning) "Looking…" else null,
                )
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = colors.surfaceContainerLow),
                ) {
                    Column(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 16.dp)) {
                        if (peers.isEmpty()) {
                            Column(
                                Modifier.fillMaxWidth()
                                    .padding(horizontal = 20.dp, vertical = 18.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                            ) {
                                Avatar(Icons.Rounded.Devices, 52.dp)
                                Spacer(Modifier.height(10.dp))
                                Text(
                                    if (running) "No one nearby yet" else "You're hidden",
                                    color = colors.onSurface,
                                    fontWeight = FontWeight.SemiBold,
                                    style = MaterialTheme.typography.bodyLarge,
                                )
                                Text(
                                    if (running)
                                        "Open AirBridge on your computer, on the same Wi‑Fi. LocalSend works too."
                                    else "Turn on visibility to send and receive.",
                                    color = colors.onSurfaceVariant,
                                    style = MaterialTheme.typography.bodyMedium,
                                    textAlign = TextAlign.Center,
                                    modifier = Modifier.padding(top = 3.dp),
                                )
                            }
                        } else {
                            val trusted = settings.trusted
                            val sorted =
                                peers.sortedWith(
                                    compareByDescending<Peer> { it.id in trusted }
                                        .thenBy { it.alias.lowercase() }
                                        .thenBy { it.id }
                                )
                            sorted.chunked(deviceColumns).forEachIndexed { row, cells ->
                                if (row > 0) Spacer(Modifier.height(10.dp))
                                Row(Modifier.fillMaxWidth()) {
                                    for (peer in cells) {
                                        key(peer.id) {
                                            PeerCell(
                                                peer,
                                                visualFor(peer, transfers, now),
                                                picking = pending != null,
                                                trusted = peer.id in trusted,
                                                modifier = Modifier.weight(1f),
                                            ) {
                                                if (vm.pending != null) vm.sendPending(peer)
                                                else menuFor = peer
                                            }
                                        }
                                    }
                                    repeat(deviceColumns - cells.size) {
                                        Spacer(Modifier.weight(1f))
                                    }
                                }
                            }
                        }
                    }
                }
                if (peers.isNotEmpty() && pending == null) {
                    Text(
                        "Tap a device to send it something.",
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(start = 16.dp, top = 7.dp),
                    )
                }

                // ------------------------------------------------------ actions
                SectionHeader("Send")
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    ActionTile("Files", Icons.Rounded.Folder, Modifier.weight(1f)) {
                        target = null
                        pickFiles.launch("*/*")
                    }
                    ActionTile("Photos", Icons.Rounded.PhotoLibrary, Modifier.weight(1f)) {
                        target = null
                        pickMedia.launch(
                            PickVisualMediaRequest(
                                ActivityResultContracts.PickVisualMedia.ImageAndVideo
                            )
                        )
                    }
                    ActionTile("Message", Icons.Rounded.ChatBubble, Modifier.weight(1f)) {
                        composeFor = null
                        composing = true
                    }
                    ActionTile("Clipboard", Icons.Rounded.ContentPaste, Modifier.weight(1f)) {
                        val clip = readClipboard()
                        if (clip != null) vm.stageText(clip)
                        else vm.toast("The clipboard is empty", good = false)
                    }
                }
                Card(modifier = Modifier.padding(top = 10.dp).fillMaxWidth()) {
                    Column(Modifier.padding(0.dp)) {
                        ListItem(
                            headlineContent = {
                                Text(
                                    "Share to any phone",
                                    color = MaterialTheme.colorScheme.onSurface,
                                )
                            },
                            supportingContent = {
                                Text("No app needed: they scan a code and open it in their browser")
                            },
                            leadingContent = {
                                Icon(
                                    Icons.Rounded.QrCode2,
                                    null,
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            },
                            modifier =
                                Modifier.clickable(onClick = { vm.screen = Screen.COMPANION }),
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        )
                        HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                        ListItem(
                            headlineContent = {
                                Text(
                                    "Pair with QR code",
                                    color = MaterialTheme.colorScheme.onSurface,
                                )
                            },
                            supportingContent = {
                                Text("Scan once, then share on any Wi‑Fi or over Tailscale")
                            },
                            leadingContent = {
                                Icon(
                                    Icons.Rounded.QrCodeScanner,
                                    null,
                                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            },
                            modifier = Modifier.clickable(onClick = { vm.screen = Screen.SCAN }),
                            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        )
                        if (vm.directNetwork != null)
                            ListItem(
                                headlineContent = {
                                    Text(
                                        "Leave Wi-Fi Direct",
                                        color = MaterialTheme.colorScheme.onSurface,
                                    )
                                },
                                supportingContent = {
                                    Text("Connected to the other phone's network")
                                },
                                leadingContent = {
                                    Icon(
                                        Icons.Rounded.WifiOff,
                                        null,
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                },
                                modifier =
                                    Modifier.clickable(onClick = { vm.leaveDirectNetwork() }),
                                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                            )
                    }
                }

                // ------------------------------------------------------- recent
                val recent = transfers.takeLast(3).reversed()
                SectionHeader("Recent", action = if (transfers.isNotEmpty()) "See all" else null) {
                    vm.screen = Screen.ACTIVITY
                }
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    colors = CardDefaults.cardColors(containerColor = colors.surfaceContainerLow),
                ) {
                    Column(Modifier.fillMaxWidth()) {
                        if (recent.isEmpty()) {
                            Text(
                                "What you send and receive shows up here.",
                                color = colors.onSurfaceVariant,
                                style = MaterialTheme.typography.bodyMedium,
                                modifier = Modifier.padding(16.dp),
                            )
                        }
                        recent.forEachIndexed { i, t ->
                            if (i > 0) HorizontalDivider(modifier = Modifier.padding(start = 72.dp))
                            RecentRow(t) { vm.screen = Screen.ACTIVITY }
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------- a device's actions
    menuFor?.let { peer ->
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = { menuFor = null },
            sheetState = sheet,
            containerColor = colors.surface,
        ) {
            Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 16.dp)) {
                Row(
                    Modifier.padding(start = 4.dp, bottom = 16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 52.dp)
                    Spacer(Modifier.width(14.dp))
                    Column {
                        Text(
                            peer.alias,
                            color = colors.onSurface,
                            fontWeight = FontWeight.SemiBold,
                            style = MaterialTheme.typography.titleLarge,
                        )
                        Text(
                            listOfNotNull(peer.deviceModel, peer.ip).joinToString(" · "),
                            color = colors.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
                val trusted = peer.id in settings.trusted
                fun act(action: () -> Unit): () -> Unit = {
                    menuFor = null
                    action()
                }
                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    ListItem(
                        headlineContent = {
                            Text("Send files", color = MaterialTheme.colorScheme.onSurface)
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.Folder,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        modifier =
                            Modifier.clickable(
                                onClick =
                                    act {
                                        target = peer
                                        pickFiles.launch("*/*")
                                    }
                            ),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text(
                                "Send photos & videos",
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.PhotoLibrary,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        modifier =
                            Modifier.clickable(
                                onClick =
                                    act {
                                        target = peer
                                        pickMedia.launch(
                                            PickVisualMediaRequest(
                                                ActivityResultContracts.PickVisualMedia
                                                    .ImageAndVideo
                                            )
                                        )
                                    }
                            ),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text("Send a message", color = MaterialTheme.colorScheme.onSurface)
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.ChatBubble,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        modifier =
                            Modifier.clickable(
                                onClick =
                                    act {
                                        composeFor = peer
                                        composing = true
                                    }
                            ),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    HorizontalDivider(modifier = Modifier.padding(start = 56.dp))
                    ListItem(
                        headlineContent = {
                            Text("Send clipboard", color = MaterialTheme.colorScheme.onSurface)
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.ContentPaste,
                                null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        },
                        modifier =
                            Modifier.clickable(
                                onClick =
                                    act {
                                        val clip = readClipboard()
                                        if (clip != null) vm.sendText(peer, clip)
                                        else vm.toast("The clipboard is empty", good = false)
                                    }
                            ),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                }
                Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                    ListItem(
                        headlineContent = {
                            Text(
                                if (trusted) "Stop trusting" else "Trust this device",
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        },
                        leadingContent = {
                            Icon(
                                Icons.Rounded.Shield,
                                null,
                                tint = if (trusted) colors.error else colors.primary,
                            )
                        },
                        modifier =
                            Modifier.clickable(
                                onClick =
                                    act {
                                        engine.setTrusted(peer.id, !trusted)
                                        vm.toast(
                                            if (trusted) "${peer.alias} is no longer trusted"
                                            else "${peer.alias} is now trusted",
                                            if (trusted) ""
                                            else "What it sends is accepted automatically",
                                        )
                                    }
                            ),
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )

                    Text(
                        if (trusted) "Your device: what it sends is accepted without asking."
                        else "Trust your own devices so what they send is accepted without asking.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
        }
    }

    // ------------------------------------------------------------ prompts
    if (composing) {
        val peer = composeFor
        TextAlert(
            title = if (peer != null) "Message ${peer.alias}" else "Send a message",
            message = "Links, codes, notes. It arrives right away.",
            initial = "",
            confirm = if (peer != null) "Send" else "Next",
            placeholder = "Type or paste…",
            singleLine = false,
            onConfirm = { text ->
                composing = false
                if (peer != null) vm.sendText(peer, text) else vm.stageText(text)
            },
            onDismiss = { composing = false },
            extra = { set ->
                TextButton(onClick = { readClipboard()?.let(set) }) { Text("Paste") }
            },
        )
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
}

/** What a device shows under its name: the latest transfer with it, alongside the device. */
private data class PeerVisual(
    val mode: RingMode,
    val progress: Float,
    val label: String?,
    val transfer: Transfer?,
)

private fun visualFor(peer: Peer, transfers: List<Transfer>, now: Long): PeerVisual {
    val mine = transfers.filter { it.peerId == peer.id }
    val active = mine.lastOrNull { !it.status.isFinal }
    val recent =
        mine.lastOrNull { it.status.isFinal && it.finished != null && now - it.finished < 4500 }
    val t = active ?: recent ?: return PeerVisual(RingMode.IDLE, 0f, null, null)
    return when (t.status) {
        Status.WAITING ->
            PeerVisual(
                RingMode.WAITING,
                0f,
                if (t.direction == Direction.OUT) "Waiting…" else "Wants to share",
                t,
            )
        Status.TRANSFERRING ->
            PeerVisual(
                RingMode.PROGRESS,
                t.progress,
                if (t.direction == Direction.OUT) "Sending…" else "Receiving…",
                t,
            )
        Status.DONE ->
            PeerVisual(
                RingMode.SUCCESS,
                1f,
                if (t.direction == Direction.OUT) "Sent" else "Received",
                t,
            )
        Status.DECLINED -> PeerVisual(RingMode.FAILED, 1f, "Declined", t)
        Status.CANCELLED -> PeerVisual(RingMode.FAILED, 1f, "Cancelled", t)
        Status.FAILED -> PeerVisual(RingMode.FAILED, 1f, "Not sent", t)
    }
}

@Composable
private fun PeerCell(
    peer: Peer,
    visual: PeerVisual,
    picking: Boolean,
    trusted: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    // Fades in; previews and screenshots show the settled state.
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { appear.animateTo(1f, tween(320)) }
    Column(
        modifier
            .graphicsLayer {
                alpha = appear.value
                val scale = 0.92f + 0.08f * appear.value
                scaleX = scale
                scaleY = scale
            }
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick)
            .padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(80.dp), contentAlignment = Alignment.Center) {
            if (picking && visual.mode == RingMode.IDLE) {
                Box(
                    Modifier.size(80.dp)
                        .clip(CircleShape)
                        .background(colors.primary.copy(alpha = 0.14f))
                )
            }
            ProgressRing(visual.mode, visual.progress, Modifier.size(80.dp))
            Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 66.dp)
        }
        Spacer(Modifier.height(6.dp))
        Text(
            peer.alias,
            color = colors.onSurface,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.Medium,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 4.dp),
        )
        val label =
            visual.label
                ?: when {
                    picking -> "Tap to send"
                    trusted -> "My device"
                    else ->
                        peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer"
                }
        val color =
            when (visual.mode) {
                RingMode.FAILED -> colors.error
                RingMode.WAITING -> colors.onSurfaceVariant
                RingMode.PROGRESS,
                RingMode.SUCCESS -> colors.primary
                RingMode.IDLE -> if (picking) colors.primary else colors.onSurfaceVariant
            }
        Text(
            label,
            color = color,
            style = MaterialTheme.typography.bodySmall,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A small gray section title, with an optional blue action or quiet note on the right. */
@Composable
private fun SectionHeader(
    title: String,
    trailing: String? = null,
    action: String? = null,
    onAction: () -> Unit = {},
) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = 48.dp)
            .padding(start = 0.dp, end = 0.dp, top = 16.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            color = colors.primary,
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.weight(1f),
        )
        if (trailing != null)
            Text(
                trailing,
                color = colors.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
            )
        if (action != null) TextButton(onClick = onAction) { Text(action) }
    }
}

/** One line of recent activity: who, what, and how it went. */
@Composable
private fun RecentRow(t: Transfer, onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier.fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box {
            val first = t.files.firstOrNull()
            Box(
                Modifier.size(40.dp)
                    .clip(RoundedCornerShape(9.dp))
                    .background(colors.surfaceContainerHighest),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (t.message != null) Icons.Rounded.ChatBubble
                    else if (first != null) fileIcon(first.mime, first.name)
                    else Icons.Rounded.AttachFile,
                    null,
                    tint = colors.primary,
                    modifier = Modifier.size(20.dp),
                )
            }
            Box(
                Modifier.align(Alignment.BottomEnd)
                    .offset(x = 4.dp, y = 4.dp)
                    .size(17.dp)
                    .clip(CircleShape)
                    .background(colors.surfaceContainerLow)
                    .padding(2.dp)
                    .clip(CircleShape)
                    .background(
                        if (t.status == Status.DONE || !t.status.isFinal) colors.primary
                        else colors.error
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (t.direction == Direction.OUT) Icons.Rounded.ArrowUpward
                    else Icons.Rounded.ArrowDownward,
                    null,
                    tint = colors.onPrimary,
                    modifier = Modifier.size(10.dp),
                )
            }
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            val title =
                when {
                    t.message != null -> t.message.replace('\n', ' ')
                    t.files.size == 1 -> t.files.first().name
                    else -> "${t.files.size} files"
                }
            Text(
                title,
                color = colors.onSurface,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            val who =
                if (t.direction == Direction.OUT) "To ${t.peerAlias}" else "From ${t.peerAlias}"
            val how =
                when (t.status) {
                    Status.WAITING ->
                        if (t.direction == Direction.OUT) "waiting…" else "waiting for you"
                    Status.TRANSFERRING -> "${(t.progress * 100).toInt()}%"
                    Status.DONE -> timeAgo(t.finished ?: t.created)
                    Status.DECLINED -> "declined"
                    Status.CANCELLED -> "cancelled"
                    Status.FAILED -> "not sent"
                }
            Text(
                "$who · $how",
                color =
                    if (t.status.isFinal && t.status != Status.DONE) colors.error
                    else colors.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}
