package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.mdhaarith.airbridge.core.Peer
import kotlinx.coroutines.delay

/** What is being shared, for the top of the sheet. */
data class SharePreview(
    val title: String,
    val subtitle: String,
    val mime: String? = null,
    val isLink: Boolean = false,
    val thumbnail: ImageBitmap? = null,
)

enum class SharePhase { PICK, LOOKING, WAITING, SENDING, DONE, FAILED }

data class ShareSheetUi(
    val preview: SharePreview,
    val peers: List<Peer>,
    val trusted: Set<String> = emptySet(),
    val targetId: String? = null,
    /** Name of the device a Direct Share target asked for, while we look for it. */
    val lookingFor: String? = null,
    val phase: SharePhase = SharePhase.PICK,
    val progress: Float = 0f,
    val error: String? = null,
) {
    val target get() = peers.firstOrNull { it.id == targetId }
    val busy get() = phase == SharePhase.WAITING || phase == SharePhase.SENDING || phase == SharePhase.LOOKING
}

private fun statusLine(ui: ShareSheetUi): String {
    val name = ui.target?.alias ?: ui.lookingFor ?: "the device"
    return when (ui.phase) {
        SharePhase.PICK -> if (ui.peers.isEmpty()) "Looking for devices…" else "Tap a device to send"
        SharePhase.LOOKING -> "Looking for $name…"
        SharePhase.WAITING -> "Waiting for $name to accept…"
        SharePhase.SENDING -> "Sending to $name · ${(ui.progress * 100).toInt()}%"
        SharePhase.DONE -> "Sent to $name"
        SharePhase.FAILED -> ui.error ?: "Couldn't send"
    }
}

/**
 * The share sheet: slides up over the app you shared from, AirDrop-style.
 * Tap a device and it sends; the sheet folds away once the other side has it.
 */
@Composable
fun ShareSheetContent(
    ui: ShareSheetUi,
    fx: Fx,
    onPick: (Peer) -> Unit,
    onDismiss: () -> Unit,
    onRetry: () -> Unit,
    onScan: () -> Unit,
) {
    val air = LocalAir.current
    val inspection = LocalInspectionMode.current
    val shown = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { shown.animateTo(1f, spring(0.78f, 260f)) }
    val positions = remember { mutableMapOf<String, Offset>() }
    var celebrated by remember { mutableStateOf(false) }
    LaunchedEffect(ui.phase) {
        if (ui.phase == SharePhase.DONE && !celebrated) {
            celebrated = true
            ui.targetId?.let { positions[it] }?.let { fx.confetti(it, 70) }
        }
    }

    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .fillMaxSize()
                .alpha(shown.value.coerceIn(0f, 1f))
                .background(Color.Black.copy(alpha = 0.5f))
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        )
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .graphicsLayer { translationY = (1f - shown.value) * size.height }
                .clip(RoundedCornerShape(topStart = 30.dp, topEnd = 30.dp))
                .background(air.solid)
                .border(1.dp, Brush.verticalGradient(listOf(Violet.copy(alpha = 0.55f), Color.Transparent)), RoundedCornerShape(topStart = 30.dp, topEnd = 30.dp))
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .navigationBarsPadding()
                .padding(start = 20.dp, end = 20.dp, top = 10.dp, bottom = 18.dp),
        ) {
            Box(Modifier.align(Alignment.CenterHorizontally).size(40.dp, 4.dp).clip(CircleShape).background(air.border))
            Spacer(Modifier.height(16.dp))
            PreviewRow(ui.preview)
            Spacer(Modifier.height(18.dp))
            Text(
                statusLine(ui),
                color = if (ui.phase == SharePhase.FAILED) Rose else air.muted,
                fontSize = 13.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.height(14.dp))
            if (ui.peers.isEmpty() && ui.phase != SharePhase.LOOKING) {
                EmptyDevices(onScan)
            } else {
                Row(
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    if (ui.phase == SharePhase.LOOKING && ui.target == null) {
                        LookingTarget(ui.lookingFor ?: "Device")
                    }
                    ui.peers.forEachIndexed { index, peer ->
                        DeviceTarget(
                            peer = peer,
                            index = index,
                            ui = ui,
                            onClick = { onPick(peer) },
                            onPositioned = { positions[peer.id] = it },
                        )
                    }
                }
            }
            if (ui.phase == SharePhase.FAILED) {
                Spacer(Modifier.height(14.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    GhostButton("Close", null, Modifier.weight(1f), height = 48.dp, onClick = onDismiss)
                    GradientButton("Try again", Icons.Rounded.Refresh, Modifier.weight(1f), height = 48.dp, onClick = onRetry)
                }
            }
        }
        FxLayer(fx, Modifier.fillMaxSize())
    }
}

@Composable
private fun PreviewRow(preview: SharePreview) {
    val air = LocalAir.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            Modifier
                .size(58.dp)
                .clip(RoundedCornerShape(16.dp))
                .background(Brush.linearGradient(listOf(Violet.copy(alpha = 0.35f), Pink.copy(alpha = 0.35f)))),
            contentAlignment = Alignment.Center,
        ) {
            val thumbnail = preview.thumbnail
            if (thumbnail != null) {
                Image(thumbnail, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                val icon = when {
                    preview.isLink -> Icons.Rounded.Link
                    preview.mime == null -> Icons.Rounded.Description
                    else -> fileIcon(preview.mime, preview.title)
                }
                Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(28.dp))
            }
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(preview.title, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(preview.subtitle, color = air.muted, fontSize = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun DeviceTarget(peer: Peer, index: Int, ui: ShareSheetUi, onClick: () -> Unit, onPositioned: (Offset) -> Unit) {
    val air = LocalAir.current
    val isTarget = peer.id == ui.targetId
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        delay(60L * index)
        appear.animateTo(1f, spring(0.5f, 420f))
    }
    val dim by animateFloatAsState(if (ui.targetId != null && !isTarget) 0.35f else 1f, tween(250), label = "dim")
    val mode = when {
        !isTarget -> RingMode.IDLE
        ui.phase == SharePhase.WAITING -> RingMode.WAITING
        ui.phase == SharePhase.SENDING -> RingMode.PROGRESS
        ui.phase == SharePhase.DONE -> RingMode.SUCCESS
        ui.phase == SharePhase.FAILED -> RingMode.FAILED
        else -> RingMode.IDLE
    }
    val subtitle = when {
        isTarget && ui.phase == SharePhase.WAITING -> "Waiting…"
        isTarget && ui.phase == SharePhase.SENDING -> "${(ui.progress * 100).toInt()}%"
        isTarget && ui.phase == SharePhase.DONE -> "Sent"
        isTarget && ui.phase == SharePhase.FAILED -> "Not sent"
        peer.id in ui.trusted -> "Trusted"
        peer.deviceModel == "Quick Share" -> "Quick Share"
        else -> peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer"
    }
    Column(
        Modifier
            .width(86.dp)
            .graphicsLayer { scaleX = appear.value; scaleY = appear.value; alpha = dim }
            .pressable(onClick, enabled = !ui.busy && ui.phase != SharePhase.DONE)
            .padding(vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier
                .size(80.dp)
                .onGloballyPositioned { onPositioned(it.boundsInRoot().center) },
            contentAlignment = Alignment.Center,
        ) {
            ProgressRing(mode, ui.progress, Modifier.fillMaxSize(), stroke = 3.5.dp)
            Avatar(peer.id, deviceIcon(peer.deviceType, peer.deviceModel), 62.dp)
        }
        Spacer(Modifier.height(6.dp))
        Text(peer.alias, color = air.text, fontSize = 13.sp, fontWeight = FontWeight.Medium, maxLines = 2, textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis)
        Text(subtitle, color = air.muted, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun LookingTarget(name: String) {
    val air = LocalAir.current
    Column(Modifier.width(86.dp).padding(vertical = 4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(80.dp), contentAlignment = Alignment.Center) {
            ProgressRing(RingMode.WAITING, 0f, Modifier.fillMaxSize(), stroke = 3.5.dp)
            Box(Modifier.size(62.dp).clip(CircleShape).background(air.surfaceStrong))
        }
        Spacer(Modifier.height(6.dp))
        Text(name, color = air.text, fontSize = 13.sp, maxLines = 2, textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis)
        Text("Looking…", color = air.muted, fontSize = 11.sp)
    }
}

@Composable
private fun EmptyDevices(onScan: () -> Unit) {
    val air = LocalAir.current
    Box(Modifier.fillMaxWidth().height(150.dp).clip(RoundedCornerShape(22.dp)).background(air.surface), contentAlignment = Alignment.Center) {
        RadarBackdrop(Modifier.fillMaxSize(), fast = true)
        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(horizontal = 20.dp)) {
            Text("No devices yet", color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
            Spacer(Modifier.height(4.dp))
            Text(
                "Open AirBridge on your computer, on the same Wi‑Fi.",
                color = air.muted,
                fontSize = 12.sp,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(10.dp))
            Row(
                Modifier.clip(CircleShape).background(air.surfaceStrong).clickable(onClick = onScan).padding(horizontal = 14.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.Refresh, contentDescription = null, tint = air.text, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text("Scan network", color = air.text, fontSize = 12.sp)
            }
        }
    }
}
