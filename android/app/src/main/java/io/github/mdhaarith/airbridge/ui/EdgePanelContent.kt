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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.res.painterResource
import io.github.mdhaarith.airbridge.R
import io.github.mdhaarith.airbridge.core.Peer
import kotlinx.coroutines.delay

/** Something the edge panel can send. */
data class EdgeAction(
    val key: String,
    val title: String,
    val subtitle: String,
    val icon: ImageVector,
    val enabled: Boolean = true,
    val thumbnail: ImageBitmap? = null,
)

data class EdgePanelUi(
    val rightSide: Boolean,
    val actions: List<EdgeAction>,
    val selected: String?,
    val peers: List<Peer>,
    val trusted: Set<String> = emptySet(),
    val targetId: String? = null,
    val phase: SharePhase = SharePhase.PICK,
    val progress: Float = 0f,
    val error: String? = null,
) {
    val busy get() = phase == SharePhase.WAITING || phase == SharePhase.SENDING
}

/**
 * The panel that slides out of the screen edge over any app: pick what to
 * send (clipboard, latest photo, something you dropped), then a device.
 */
@Composable
fun EdgePanelContent(
    ui: EdgePanelUi,
    fx: Fx,
    onSelect: (String) -> Unit,
    onPick: (Peer) -> Unit,
    onClose: () -> Unit,
) {
    val air = LocalAir.current
    val inspection = LocalInspectionMode.current
    val shown = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { shown.animateTo(1f, spring(0.72f, 300f)) }
    val positions = remember { mutableMapOf<String, Offset>() }
    var celebrated by remember { mutableStateOf(false) }
    LaunchedEffect(ui.phase) {
        if (ui.phase == SharePhase.DONE && !celebrated) {
            celebrated = true
            ui.targetId?.let { positions[it] }?.let { fx.confetti(it, 60) }
        }
    }
    val sign = if (ui.rightSide) 1f else -1f
    val shape = if (ui.rightSide) RoundedCornerShape(topStart = 30.dp, bottomStart = 30.dp, topEnd = 8.dp, bottomEnd = 8.dp)
    else RoundedCornerShape(topEnd = 30.dp, bottomEnd = 30.dp, topStart = 8.dp, bottomStart = 8.dp)

    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .fillMaxSize()
                .alpha(shown.value.coerceIn(0f, 1f))
                .background(Brush.horizontalGradient(
                    if (ui.rightSide) listOf(Color.Transparent, Color.Black.copy(alpha = 0.55f))
                    else listOf(Color.Black.copy(alpha = 0.55f), Color.Transparent),
                ))
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onClose),
        )
        Column(
            Modifier
                .align(if (ui.rightSide) Alignment.CenterEnd else Alignment.CenterStart)
                .padding(vertical = 40.dp)
                .width(300.dp)
                .heightIn(max = 640.dp)
                .graphicsLayer { translationX = sign * (1f - shown.value) * (size.width + 40f) }
                .clip(shape)
                .background(air.solid)
                .border(1.dp, Brush.verticalGradient(listOf(Violet.copy(alpha = 0.7f), Pink.copy(alpha = 0.4f))), shape)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .verticalScroll(rememberScrollState())
                .padding(18.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Image(painterResource(R.drawable.ic_logo), contentDescription = null, modifier = Modifier.size(30.dp))
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    Text("AirBridge", color = air.text, fontWeight = FontWeight.Bold, fontSize = 16.sp)
                    Text(
                        when (ui.phase) {
                            SharePhase.WAITING -> "Waiting for the other side…"
                            SharePhase.SENDING -> "Sending · ${(ui.progress * 100).toInt()}%"
                            SharePhase.DONE -> "Sent"
                            SharePhase.FAILED -> ui.error ?: "Couldn't send"
                            else -> "Send to a nearby device"
                        },
                        color = if (ui.phase == SharePhase.FAILED) Rose else air.muted,
                        fontSize = 12.sp,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Box(Modifier.size(32.dp).clip(CircleShape).background(air.surface).clickable(onClick = onClose), contentAlignment = Alignment.Center) {
                    Icon(Icons.Rounded.Close, contentDescription = "Close", tint = air.muted, modifier = Modifier.size(18.dp))
                }
            }
            Spacer(Modifier.height(16.dp))
            Text("SEND", color = air.faint, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 1.2.sp)
            Spacer(Modifier.height(8.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                ui.actions.forEach { action ->
                    ActionChip(action, selected = action.key == ui.selected, enabled = action.enabled && !ui.busy) { onSelect(action.key) }
                }
            }
            Spacer(Modifier.height(18.dp))
            Text("TO", color = air.faint, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 1.2.sp)
            Spacer(Modifier.height(6.dp))
            if (ui.peers.isEmpty()) {
                Text("Looking for devices… Open AirBridge on your computer.", color = air.muted, fontSize = 13.sp, modifier = Modifier.padding(vertical = 10.dp))
            }
            ui.peers.forEachIndexed { index, peer ->
                DeviceRow(peer, index, ui, onClick = { onPick(peer) }, onPositioned = { positions[peer.id] = it })
            }
        }
        FxLayer(fx, Modifier.fillMaxSize())
    }
}

@Composable
private fun ActionChip(action: EdgeAction, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val air = LocalAir.current
    val highlight by animateFloatAsState(if (selected) 1f else 0f, tween(200), label = "chip")
    Row(
        Modifier
            .fillMaxWidth()
            .alpha(if (action.enabled) 1f else 0.5f)
            .clip(RoundedCornerShape(16.dp))
            .background(air.surface)
            .background(Brush.horizontalGradient(listOf(Violet.copy(alpha = 0.28f * highlight), Pink.copy(alpha = 0.18f * highlight))))
            .border(1.dp, if (selected) Violet.copy(alpha = 0.8f) else air.border, RoundedCornerShape(16.dp))
            .pressable(onClick, enabled = enabled)
            .padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(12.dp)).background(air.surfaceStrong), contentAlignment = Alignment.Center) {
            val thumbnail = action.thumbnail
            if (thumbnail != null) Image(thumbnail, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            else Icon(action.icon, contentDescription = null, tint = if (selected) Color.White else air.muted, modifier = Modifier.size(20.dp))
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(action.title, color = air.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
            Text(action.subtitle, color = air.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun DeviceRow(peer: Peer, index: Int, ui: EdgePanelUi, onClick: () -> Unit, onPositioned: (Offset) -> Unit) {
    val air = LocalAir.current
    val isTarget = peer.id == ui.targetId
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        delay(80L + 55L * index)
        appear.animateTo(1f, spring(0.55f, 380f))
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
    Row(
        Modifier
            .fillMaxWidth()
            .graphicsLayer {
                alpha = dim * appear.value.coerceIn(0f, 1f)
                translationX = (1f - appear.value) * 60f
            }
            .clip(RoundedCornerShape(18.dp))
            .pressable(onClick, enabled = ui.selected != null && !ui.busy && ui.phase != SharePhase.DONE)
            .padding(vertical = 6.dp, horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(58.dp).onGloballyPositioned { onPositioned(it.boundsInRoot().center) }, contentAlignment = Alignment.Center) {
            ProgressRing(mode, ui.progress, Modifier.fillMaxSize(), stroke = 3.dp)
            Avatar(peer.id, deviceIcon(peer.deviceType, peer.deviceModel), 46.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(peer.alias, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                when {
                    isTarget && ui.phase == SharePhase.WAITING -> "Waiting for accept…"
                    isTarget && ui.phase == SharePhase.SENDING -> "${(ui.progress * 100).toInt()}%"
                    isTarget && ui.phase == SharePhase.DONE -> "Sent"
                    isTarget && ui.phase == SharePhase.FAILED -> "Not sent"
                    peer.id in ui.trusted -> "Trusted"
                    else -> peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer"
                },
                color = air.muted,
                fontSize = 12.sp,
                maxLines = 1,
            )
        }
    }
}
