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
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.Folder
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Schedule
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
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
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Discovery
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.service.AirBridgeService
import kotlinx.coroutines.delay

@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun HomeScreen(vm: AppViewModel) {
    val air = LocalAir.current
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
            if (event is EngineEvent.Finished && event.transfer.status == Status.DONE) haptics.performHapticFeedback(HapticFeedbackType.LongPress)
        }
    }

    var menuFor by remember { mutableStateOf<Peer?>(null) }
    var composeFor by remember { mutableStateOf<Peer?>(null) }
    var composing by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf(false) }
    var target by remember { mutableStateOf<Peer?>(null) }

    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        val peer = target
        target = null
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        if (peer != null) vm.sendFiles(peer, uris) else vm.stage(uris)
    }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { uris ->
        val peer = target
        target = null
        if (uris.isEmpty()) return@rememberLauncherForActivityResult
        if (peer != null) vm.sendFiles(peer, uris) else vm.stage(uris)
    }
    fun readClipboard(): String? =
        (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).primaryClip
            ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()?.takeIf { it.isNotBlank() }

    val activeCount = transfers.count { !it.status.isFinal }
    val pending = vm.pending

    Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
        // ------------------------------------------------- large title bar
        Row(Modifier.fillMaxWidth().padding(start = 8.dp, end = 4.dp, top = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            Spacer(Modifier.weight(1f))
            val spin = rememberInfiniteTransition(label = "scan").animateFloat(0f, 360f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "s")
            BarButton(Icons.Rounded.Refresh, "Look for devices", Modifier.graphicsLayer { rotationZ = if (vm.scanning) spin.value else 0f }) { vm.scan() }
            BarButton(Icons.Rounded.Schedule, "Activity", badge = activeCount) { vm.screen = Screen.ACTIVITY }
            BarButton(Icons.Rounded.Settings, "Settings") { vm.screen = Screen.SETTINGS }
        }
        Text("AirBridge", color = air.text, fontWeight = FontWeight.Bold, fontSize = 34.sp, modifier = Modifier.padding(start = 16.dp))

        // ------------------------------------------------------- devices
        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
            if (peers.isEmpty()) {
                Column(Modifier.padding(horizontal = 36.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(if (running) "No one nearby yet" else "You're hidden", color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
                    Text(
                        if (running) "Open AirBridge on your computer, on the same Wi‑Fi. LocalSend works too." else "Turn on visibility to send and receive.",
                        color = air.muted, fontSize = 15.sp, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp),
                    )
                }
            } else {
                FlowRow(
                    Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterHorizontally),
                    verticalArrangement = Arrangement.spacedBy(18.dp),
                ) {
                    val trusted = settings.trusted
                    for (peer in peers.sortedWith(compareBy({ it.alias.lowercase() }, { it.id }))) {
                        key(peer.id) {
                            PeerCell(peer, visualFor(peer, transfers, now), picking = pending != null, trusted = peer.id in trusted) {
                                if (vm.pending != null) vm.sendPending(peer) else menuFor = peer
                            }
                        }
                    }
                }
            }
        }

        // ------------------------------------------------ what is waiting
        AnimatedVisibility(
            pending != null,
            enter = slideInVertically(tween(300)) { it / 2 } + fadeIn(tween(200)),
            exit = slideOutVertically(tween(250)) { it / 2 } + fadeOut(tween(200)),
        ) {
            val share = pending ?: PendingShare()
            Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp), padding = 12.dp) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(42.dp).clip(RoundedCornerShape(9.dp)).background(air.fill), contentAlignment = Alignment.Center) {
                        Icon(if (share.text != null) Icons.Rounded.ChatBubble else Icons.Rounded.AttachFile, null, tint = air.accent)
                    }
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(share.label, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            (if (share.text == null) "${formatBytes(share.totalBytes)} · " else "") + if (peers.isEmpty()) "waiting for a device…" else "choose a device",
                            color = air.muted, fontSize = 13.sp,
                        )
                    }
                    Box(Modifier.size(30.dp).clip(CircleShape).background(air.fill).pressable({ vm.clearPending() }), contentAlignment = Alignment.Center) {
                        Icon(Icons.Rounded.Close, "Clear", tint = air.muted, modifier = Modifier.size(16.dp))
                    }
                }
            }
        }

        // ---------------------------------------------------- visible as
        if (pending == null) {
            val ip = remember(running, peers.size) { Discovery.localAddresses().firstOrNull()?.address?.hostAddress }
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
                Beacon(running, size = 50.dp)
                Row(
                    Modifier.clip(RoundedCornerShape(8.dp)).pressable({ if (running) renaming = true else AirBridgeService.start(context) }).padding(horizontal = 10.dp, vertical = 3.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    StatusDot(running)
                    Spacer(Modifier.width(7.dp))
                    if (running) {
                        Text("Visible as ", color = air.muted, fontSize = 15.sp)
                        Text(settings.alias, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 200.dp))
                    } else {
                        Text("Hidden · tap to become visible", color = air.muted, fontSize = 15.sp)
                    }
                }
                if (running && ip != null) Text(ip, color = air.faint, fontSize = 12.sp)
            }
        }

        // ------------------------------------------------------- actions
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ActionTile("Files", Icons.Rounded.Folder, Modifier.weight(1f)) {
                target = null
                pickFiles.launch("*/*")
            }
            ActionTile("Photos", Icons.Rounded.PhotoLibrary, Modifier.weight(1f)) {
                target = null
                pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
            }
            ActionTile("Message", Icons.Rounded.ChatBubble, Modifier.weight(1f)) {
                composeFor = null
                composing = true
            }
            ActionTile("Clipboard", Icons.Rounded.ContentPaste, Modifier.weight(1f)) {
                val clip = readClipboard()
                if (clip != null) vm.stageText(clip) else vm.toast("The clipboard is empty", good = false)
            }
        }
    }

    // ------------------------------------------------- a device's actions
    menuFor?.let { peer ->
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = { menuFor = null },
            sheetState = sheet,
            containerColor = air.bg,
            dragHandle = { Box(Modifier.padding(top = 8.dp, bottom = 10.dp).size(36.dp, 5.dp).clip(CircleShape).background(air.faint)) },
        ) {
            Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 16.dp)) {
                Row(Modifier.padding(start = 4.dp, bottom = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 52.dp)
                    Spacer(Modifier.width(14.dp))
                    Column {
                        Text(peer.alias, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 20.sp)
                        Text(listOfNotNull(peer.deviceModel, peer.ip).joinToString(" · "), color = air.muted, fontSize = 14.sp)
                    }
                }
                val trusted = peer.id in settings.trusted
                fun act(action: () -> Unit): () -> Unit = { menuFor = null; action() }
                Group {
                    ListRow("Send Files", icon = Icons.Rounded.Folder, chevron = false, onClick = act { target = peer; pickFiles.launch("*/*") })
                    RowSeparator(59.dp)
                    ListRow("Send Photos & Videos", icon = Icons.Rounded.PhotoLibrary, chevron = false, onClick = act {
                        target = peer
                        pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                    })
                    RowSeparator(59.dp)
                    ListRow("Send a Message", icon = Icons.Rounded.ChatBubble, chevron = false, onClick = act { composeFor = peer; composing = true })
                    RowSeparator(59.dp)
                    ListRow("Send Clipboard", icon = Icons.Rounded.ContentPaste, chevron = false, onClick = act {
                        val clip = readClipboard()
                        if (clip != null) vm.sendText(peer, clip) else vm.toast("The clipboard is empty", good = false)
                    })
                }
                Group(footer = if (trusted) "Your device: what it sends is accepted without asking." else "Trust your own devices so what they send is accepted without asking.") {
                    ListRow(
                        if (trusted) "Stop Trusting" else "Trust This Device",
                        icon = Icons.Rounded.Shield, iconColor = if (trusted) air.danger else air.success, chevron = false,
                        onClick = act {
                            engine.setTrusted(peer.id, !trusted)
                            vm.toast(if (trusted) "${peer.alias} is no longer trusted" else "${peer.alias} is now trusted", if (trusted) "" else "What it sends is accepted automatically")
                        },
                    )
                }
            }
        }
    }

    // ------------------------------------------------------------ prompts
    if (composing) {
        val peer = composeFor
        TextAlert(
            title = if (peer != null) "Message ${peer.alias}" else "Send a Message",
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
                Text(
                    "Paste", color = air.accent, fontSize = 15.sp,
                    modifier = Modifier.padding(top = 8.dp).pressable({ readClipboard()?.let(set) }),
                )
            },
        )
    }
    if (renaming) {
        TextAlert(
            title = "Device Name",
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

/** What a device shows under its name: the latest transfer with it, like the iOS share sheet. */
private data class PeerVisual(val mode: RingMode, val progress: Float, val label: String?, val transfer: Transfer?)

private fun visualFor(peer: Peer, transfers: List<Transfer>, now: Long): PeerVisual {
    val mine = transfers.filter { it.peerId == peer.id }
    val active = mine.lastOrNull { !it.status.isFinal }
    val recent = mine.lastOrNull { it.status.isFinal && it.finished != null && now - it.finished < 4500 }
    val t = active ?: recent ?: return PeerVisual(RingMode.IDLE, 0f, null, null)
    return when (t.status) {
        Status.WAITING -> PeerVisual(RingMode.WAITING, 0f, if (t.direction == Direction.OUT) "Waiting…" else "Wants to share", t)
        Status.TRANSFERRING -> PeerVisual(RingMode.PROGRESS, t.progress, if (t.direction == Direction.OUT) "Sending…" else "Receiving…", t)
        Status.DONE -> PeerVisual(RingMode.SUCCESS, 1f, if (t.direction == Direction.OUT) "Sent" else "Received", t)
        Status.DECLINED -> PeerVisual(RingMode.FAILED, 1f, "Declined", t)
        Status.CANCELLED -> PeerVisual(RingMode.FAILED, 1f, "Cancelled", t)
        Status.FAILED -> PeerVisual(RingMode.FAILED, 1f, "Not Sent", t)
    }
}

@Composable
private fun PeerCell(peer: Peer, visual: PeerVisual, picking: Boolean, trusted: Boolean, onClick: () -> Unit) {
    val air = LocalAir.current
    // Fades in; previews and screenshots show the settled state.
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { appear.animateTo(1f, tween(320)) }
    Column(
        Modifier
            .width(104.dp)
            .graphicsLayer {
                alpha = appear.value
                val scale = 0.92f + 0.08f * appear.value
                scaleX = scale
                scaleY = scale
            }
            .clip(RoundedCornerShape(12.dp))
            .pressable(onClick)
            .padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(80.dp), contentAlignment = Alignment.Center) {
            if (picking && visual.mode == RingMode.IDLE) {
                Box(Modifier.size(80.dp).clip(CircleShape).background(air.accent.copy(alpha = 0.14f)))
            }
            ProgressRing(visual.mode, visual.progress, Modifier.size(80.dp))
            Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 66.dp)
        }
        Spacer(Modifier.height(6.dp))
        Text(peer.alias, color = air.text, fontSize = 13.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.Center)
        val label = visual.label ?: when {
            picking -> "Tap to send"
            trusted -> "My device"
            else -> peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer"
        }
        val color = when (visual.mode) {
            RingMode.FAILED -> air.danger
            RingMode.WAITING -> air.muted
            RingMode.PROGRESS, RingMode.SUCCESS -> air.accent
            RingMode.IDLE -> if (picking) air.accent else air.muted
        }
        Text(label, color = color, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
