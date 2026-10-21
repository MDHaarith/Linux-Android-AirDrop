package io.github.mdhaarith.airbridge.ui

import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.animateOffsetAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.rounded.AttachFile
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.History
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.PhotoLibrary
import androidx.compose.material.icons.rounded.Radar
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Shield
import androidx.compose.material.icons.rounded.Star
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.R
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Discovery
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.service.AirBridgeService
import kotlinx.coroutines.delay
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(vm: AppViewModel, fx: Fx) {
    val air = LocalAir.current
    val context = LocalContext.current
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
        // ------------------------------------------------------------ top bar
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Image(painterResource(R.drawable.ic_logo), null, Modifier.size(34.dp))
            Spacer(Modifier.width(10.dp))
            Text("AirBridge", color = air.text, fontWeight = FontWeight.Bold, fontSize = 20.sp, modifier = Modifier.weight(1f))
            val spin = rememberInfiniteTransition(label = "scan").animateFloat(0f, 360f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "s")
            IconCircleButton(Icons.Rounded.Radar, "Scan network", Modifier.graphicsLayer { rotationZ = if (vm.scanning) spin.value else 0f }) { vm.scan() }
            Spacer(Modifier.width(6.dp))
            IconCircleButton(Icons.Rounded.History, "Activity", badge = activeCount) { vm.screen = Screen.ACTIVITY }
            Spacer(Modifier.width(6.dp))
            IconCircleButton(Icons.Rounded.Settings, "Settings") { vm.screen = Screen.SETTINGS }
        }

        // ------------------------------------------------------------- me chip
        val ip = remember(running, peers.size) { Discovery.localAddresses().firstOrNull()?.address?.hostAddress }
        Row(
            Modifier
                .align(Alignment.CenterHorizontally)
                .clip(RoundedCornerShape(50))
                .background(air.surface)
                .border(1.dp, air.border, RoundedCornerShape(50))
                .pressable({ if (running) renaming = true else AirBridgeService.start(context) })
                .padding(horizontal = 14.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PulseDot(running)
            Spacer(Modifier.width(8.dp))
            if (running) {
                Text("Visible as ", color = air.muted, fontSize = 14.sp)
                Text(settings.alias, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 180.dp))
                if (ip != null) Text("  $ip", color = air.faint, fontSize = 13.sp)
            } else {
                Text("Hidden · tap to become visible", color = air.muted, fontSize = 14.sp)
            }
        }

        // --------------------------------------------------------------- radar
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Radar(
                peers = peers,
                transfers = transfers,
                now = now,
                picking = pending != null,
                fx = fx,
                meId = engine.identity.fingerprint,
                onPeer = { peer -> if (vm.pending != null) vm.sendPending(peer) else menuFor = peer },
                vm = vm,
            )
            if (peers.isEmpty()) {
                Column(
                    Modifier.align(Alignment.BottomCenter).padding(horizontal = 28.dp, vertical = 8.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(if (running) "Looking for nearby devices…" else "You're hidden", color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
                    Text(
                        "Open AirBridge on your computer (same Wi‑Fi). LocalSend works too.",
                        color = air.muted, fontSize = 14.sp, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 4.dp),
                    )
                }
            }
        }

        // ------------------------------------------------------- pending tray
        AnimatedVisibility(
            pending != null,
            enter = slideInVertically(spring(0.7f, 380f)) { it } + fadeIn(),
            exit = slideOutVertically { it } + fadeOut(),
        ) {
            val share = pending ?: PendingShare()
            GlassCard(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(Modifier.size(42.dp).clip(RoundedCornerShape(13.dp)).background(AccentGradient), contentAlignment = Alignment.Center) {
                        Icon(if (share.text != null) Icons.Rounded.ChatBubble else Icons.Rounded.AttachFile, null, tint = Color.White)
                    }
                    Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(share.label, color = air.text, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(
                            (if (share.text == null) "${formatBytes(share.totalBytes)} · " else "") + if (peers.isEmpty()) "waiting for a device…" else "tap a device to send",
                            color = air.muted, fontSize = 13.sp,
                        )
                    }
                    IconCircleButton(Icons.Rounded.Close, "Clear") { vm.clearPending() }
                }
            }
        }

        // ------------------------------------------------------------- actions
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            GradientButton("Send files", Icons.Rounded.AttachFile, Modifier.weight(1.35f)) {
                target = null
                pickFiles.launch("*/*")
            }
            GhostButton("Photos", Icons.Rounded.PhotoLibrary, Modifier.weight(1f)) {
                target = null
                pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
            }
            GhostButton("Text", Icons.Rounded.ChatBubble, Modifier.weight(0.9f)) {
                composeFor = null
                composing = true
            }
        }
    }

    // -------------------------------------------------------- peer actions
    menuFor?.let { peer ->
        val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(onDismissRequest = { menuFor = null }, sheetState = sheet, containerColor = air.solid) {
            Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 20.dp).padding(bottom = 16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Avatar(peer.id, deviceIcon(peer.deviceType, peer.deviceModel), 52.dp)
                    Spacer(Modifier.width(14.dp))
                    Column {
                        Text(peer.alias, color = air.text, fontWeight = FontWeight.Bold, fontSize = 18.sp)
                        Text(listOfNotNull(peer.deviceModel, peer.ip).joinToString(" · "), color = air.muted, fontSize = 13.sp)
                    }
                }
                Spacer(Modifier.height(14.dp))
                val trusted = peer.id in settings.trusted
                val actions = listOf(
                    Triple(Icons.Rounded.AttachFile, "Send files") { target = peer; pickFiles.launch("*/*") },
                    Triple(Icons.Rounded.PhotoLibrary, "Send photos & videos") {
                        target = peer
                        pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                    },
                    Triple(Icons.Rounded.ChatBubble, "Send a message") { composeFor = peer; composing = true },
                    Triple(Icons.Rounded.ContentPaste, "Send clipboard") {
                        val clip = readClipboard()
                        if (clip != null) vm.sendText(peer, clip) else vm.toast("The clipboard is empty", good = false)
                    },
                    Triple(Icons.Rounded.Shield, if (trusted) "Stop trusting" else "Trust this device") {
                        engine.setTrusted(peer.id, !trusted)
                        vm.toast(if (trusted) "${peer.alias} is no longer trusted" else "${peer.alias} is now trusted", if (trusted) "" else "Its transfers are accepted automatically")
                    },
                )
                for ((icon, label, action) in actions) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(14.dp))
                            .pressable({ menuFor = null; action() })
                            .padding(horizontal = 8.dp, vertical = 13.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(icon, null, tint = Cyan)
                        Spacer(Modifier.width(14.dp))
                        Text(label, color = air.text, fontSize = 16.sp)
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ dialogs
    if (composing) {
        var text by remember { mutableStateOf("") }
        val peer = composeFor
        AlertDialog(
            onDismissRequest = { composing = false },
            title = { Text(if (peer != null) "Message ${peer.alias}" else "Send a message") },
            text = {
                Column {
                    OutlinedTextField(text, { text = it }, Modifier.fillMaxWidth().height(160.dp), placeholder = { Text("Links, codes, notes…") })
                    TextButton({ readClipboard()?.let { text = it } }) {
                        Icon(Icons.Rounded.ContentPaste, null, Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Paste")
                    }
                }
            },
            confirmButton = {
                TextButton({
                    composing = false
                    if (peer != null) vm.sendText(peer, text) else vm.stageText(text)
                }, enabled = text.isNotBlank()) {
                    Icon(Icons.AutoMirrored.Rounded.Send, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(if (peer != null) "Send" else "Choose device")
                }
            },
            dismissButton = { TextButton({ composing = false }) { Text("Cancel") } },
        )
    }
    if (renaming) {
        var name by remember { mutableStateOf(settings.alias) }
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text("Device name") },
            text = {
                Column {
                    Text("This is how your computer sees this phone.", color = air.muted)
                    Spacer(Modifier.height(10.dp))
                    OutlinedTextField(name, { name = it.take(64) }, singleLine = true)
                }
            },
            confirmButton = { TextButton({ renaming = false; engine.settings.update { it.copy(alias = name) } }) { Text("Save") } },
            dismissButton = { TextButton({ renaming = false }) { Text("Cancel") } },
        )
    }
}

/** What a peer bubble shows: the latest transfer with that device. */
private data class PeerVisual(val mode: RingMode, val progress: Float, val label: String?, val transfer: Transfer?)

private fun visualFor(peer: Peer, transfers: List<Transfer>, now: Long): PeerVisual {
    val mine = transfers.filter { it.peerId == peer.id }
    val active = mine.lastOrNull { !it.status.isFinal }
    val recent = mine.lastOrNull { it.status.isFinal && it.finished != null && now - it.finished < 4500 }
    val t = active ?: recent ?: return PeerVisual(RingMode.IDLE, 0f, null, null)
    return when (t.status) {
        Status.WAITING -> PeerVisual(RingMode.WAITING, 0f, if (t.direction == Direction.OUT) "Waiting…" else "Wants to share…", t)
        Status.TRANSFERRING -> PeerVisual(RingMode.PROGRESS, t.progress, "${(t.progress * 100).roundToInt()}% · ${formatBytes(t.speed)}/s", t)
        Status.DONE -> PeerVisual(RingMode.SUCCESS, 1f, if (t.direction == Direction.OUT) (if (t.message != null) "Message sent" else "Sent") else "Received", t)
        Status.DECLINED -> PeerVisual(RingMode.FAILED, 1f, "Declined", t)
        Status.CANCELLED -> PeerVisual(RingMode.FAILED, 1f, "Cancelled", t)
        Status.FAILED -> PeerVisual(RingMode.FAILED, 1f, "Failed", t)
    }
}

@Composable
private fun Radar(
    peers: List<Peer>,
    transfers: List<Transfer>,
    now: Long,
    picking: Boolean,
    fx: Fx,
    meId: String,
    onPeer: (Peer) -> Unit,
    vm: AppViewModel,
) {
    val density = LocalDensity.current
    val haptics = LocalHapticFeedback.current
    val positions = remember { mutableMapOf<String, Offset>() }
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val w = constraints.maxWidth.toFloat()
        val h = constraints.maxHeight.toFloat()
        val center = Offset(w / 2, h / 2)
        val side = min(maxWidth.value, maxHeight.value).dp
        Box(Modifier.size(side * 1.05f)) { RadarBackdrop(fast = picking) }

        // This device in the middle.
        val meGlow = rememberInfiniteTransition(label = "me").animateFloat(0.9f, 1.15f, infiniteRepeatable(tween(2000), RepeatMode.Reverse), label = "g")
        Canvas(Modifier.size(170.dp)) {
            drawCircle(Brush.radialGradient(listOf(Violet.copy(alpha = 0.45f), Color.Transparent)), size.minDimension / 2 * meGlow.value)
        }
        Avatar(meId, Icons.Rounded.PhoneAndroid, 92.dp, colors = Color(0xFF8F7DFF) to Pink)

        // Devices around it: up to six on an inner ellipse, the rest further out.
        val sorted = peers.sortedWith(compareBy({ it.alias.lowercase() }, { it.id }))
        val inner = min(sorted.size, 6)
        val outer = sorted.size - inner
        val rx = min(w * 0.3f, with(density) { 160.dp.toPx() })
        val ry = min(h * 0.33f, rx * 1.25f)
        sorted.forEachIndexed { i, peer ->
            val onInner = i < inner
            val count = if (onInner) inner else outer
            val idx = if (onInner) i else i - inner
            // Start at the upper left, so 3 devices sit left/right/below like a triangle.
            val start = if (onInner) -2.6 else -2.6 + PI / count
            val angle = start + idx.toDouble() / count * 2 * PI
            val scale = if (onInner) 1f else 1.4f
            positions[peer.id] = Offset((cos(angle) * rx * scale).toFloat(), (sin(angle) * ry * scale).toFloat())
        }
        positions.keys.retainAll(sorted.map { it.id }.toSet())

        for (peer in sorted) {
            key(peer.id) {
                PeerBubble(peer, positions[peer.id] ?: Offset.Zero, visualFor(peer, transfers, now), picking, peer.id in vm.engine.settings.value.trusted) { onPeer(peer) }
            }
        }

        // Particle streams while bytes flow.
        val active = transfers.filter { it.status == Status.TRANSFERRING && it.peerId in positions }
        SideEffect {
            for (t in active) {
                val toPeer = { positions[t.peerId]?.let { center + it } }
                val toMe = { center }
                if (t.direction == Direction.OUT) fx.stream(t.id, toMe, toPeer, Violet) else fx.stream(t.id, toPeer, toMe, Cyan)
            }
            fx.keepOnly(active.map { it.id }.toSet())
        }
        LaunchedEffect(Unit) {
            vm.engine.events.collect { event ->
                if (event is EngineEvent.Finished && event.transfer.status == Status.DONE) {
                    val at = positions[event.transfer.peerId]?.let { center + it } ?: center
                    if (event.transfer.message == null || event.transfer.direction == Direction.OUT) fx.confetti(at, if (event.transfer.direction == Direction.OUT) 70 else 110)
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                }
            }
        }
        FxLayer(fx, Modifier.fillMaxSize())
    }
}

@Composable
private fun PeerBubble(peer: Peer, position: Offset, visual: PeerVisual, picking: Boolean, trusted: Boolean, onClick: () -> Unit) {
    val air = LocalAir.current
    val density = LocalDensity.current
    val animated by animateOffsetAsState(position, spring(0.62f, 110f), label = "pos")
    // Pops in with a spring; previews/screenshots show the settled state.
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { appear.animateTo(1f, spring(0.5f, 260f)) }
    val phase = remember(peer.id) { (peer.id.hashCode() and 0xfff) % 1400 }
    val bob by rememberInfiniteTransition(label = "bob").animateFloat(-1f, 1f, infiniteRepeatable(tween(2800 + phase), RepeatMode.Reverse), label = "b")
    val success = visual.mode == RingMode.SUCCESS
    val checkScale by animateFloatAsState(if (success) 1f else 0f, spring(0.45f, 400f), label = "check")
    val failed = visual.mode == RingMode.FAILED
    val shake = remember { Animatable(0f) }
    LaunchedEffect(failed) {
        if (failed) for (x in listOf(10f, -10f, 7f, -7f, 3f, 0f)) shake.animateTo(x, tween(55))
    }
    val bubbleHalf = with(density) { 42.dp.toPx() }
    val boxHalfHeight = with(density) { 70.dp.toPx() }

    Column(
        Modifier
            .width(130.dp)
            .height(140.dp)
            .offset {
                IntOffset(
                    (animated.x + shake.value).roundToInt(),
                    (animated.y + boxHalfHeight - bubbleHalf + bob * 6.dp.toPx()).roundToInt(),
                )
            }
            .graphicsLayer {
                scaleX = appear.value
                scaleY = appear.value
                alpha = appear.value.coerceIn(0f, 1f)
            }
            .pressable(onClick),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(84.dp), contentAlignment = Alignment.Center) {
            if (picking) {
                val spin = rememberInfiniteTransition(label = "pick").animateFloat(0f, 360f, infiniteRepeatable(tween(5000, easing = LinearEasing)), label = "p")
                Canvas(Modifier.size(84.dp).graphicsLayer { rotationZ = spin.value }) {
                    drawCircle(Cyan, size.minDimension / 2 - 2f, style = Stroke(3f, pathEffect = androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(14f, 12f))))
                }
            }
            ProgressRing(visual.mode, visual.progress, Modifier.size(84.dp))
            Avatar(peer.id, deviceIcon(peer.deviceType, peer.deviceModel), 68.dp)
            Box(
                Modifier.size(68.dp).graphicsLayer { scaleX = checkScale; scaleY = checkScale; alpha = checkScale }
                    .clip(CircleShape).background(Mint),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(34.dp))
            }
            if (trusted) {
                Box(
                    Modifier.align(Alignment.TopEnd).size(22.dp).clip(CircleShape).background(Amber),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Rounded.Star, null, tint = Color(0xFF3A2A00), modifier = Modifier.size(14.dp)) }
            }
        }
        Spacer(Modifier.height(6.dp))
        Text(peer.alias, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        val label = visual.label ?: if (picking) "Tap to send" else peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer"
        val color = when (visual.mode) {
            RingMode.SUCCESS -> Mint
            RingMode.FAILED -> Rose
            RingMode.PROGRESS, RingMode.WAITING -> Cyan
            RingMode.IDLE -> if (picking) Cyan else air.muted
        }
        Text(label, color = color, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
    }
}
