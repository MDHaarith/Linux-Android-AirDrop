package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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
    onSelect: (String) -> Unit,
    onPick: (Peer) -> Unit,
    onClose: () -> Unit,
) {
    val air = LocalAir.current
    val inspection = LocalInspectionMode.current
    val shown = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { shown.animateTo(1f, tween(360, easing = CubicBezierEasing(0.16f, 1f, 0.3f, 1f))) }
    val sign = if (ui.rightSide) 1f else -1f

    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .fillMaxSize()
                .alpha(shown.value.coerceIn(0f, 1f))
                .background(Color.Black.copy(alpha = 0.2f))
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onClose),
        )
        Column(
            Modifier
                .align(if (ui.rightSide) Alignment.CenterEnd else Alignment.CenterStart)
                .padding(vertical = 40.dp, horizontal = 10.dp)
                .width(296.dp)
                .heightIn(max = 640.dp)
                .graphicsLayer { translationX = sign * (1f - shown.value) * (size.width + 60f) }
                .shadow(28.dp, RoundedCornerShape(22.dp))
                .clip(RoundedCornerShape(22.dp))
                .background(air.bg)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .verticalScroll(rememberScrollState())
                .padding(14.dp),
        ) {
            Row(Modifier.padding(start = 2.dp, bottom = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                Image(painterResource(R.drawable.ic_logo), contentDescription = null, modifier = Modifier.size(28.dp))
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    Text("AirBridge", color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 16.sp)
                    Text(
                        when (ui.phase) {
                            SharePhase.WAITING -> "Waiting for the other side…"
                            SharePhase.SENDING -> "Sending…"
                            SharePhase.DONE -> "Sent"
                            SharePhase.FAILED -> ui.error ?: "Couldn't send"
                            else -> "Send to a nearby device"
                        },
                        color = when (ui.phase) {
                            SharePhase.FAILED -> air.danger
                            SharePhase.DONE, SharePhase.SENDING -> air.accent
                            else -> air.muted
                        },
                        fontSize = 12.sp,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Box(Modifier.size(28.dp).clip(CircleShape).background(air.fill).clickable(onClick = onClose), contentAlignment = Alignment.Center) {
                    Icon(Icons.Rounded.Close, contentDescription = "Close", tint = air.muted, modifier = Modifier.size(16.dp))
                }
            }
            Text("SEND", color = air.muted, fontSize = 12.sp, modifier = Modifier.padding(start = 14.dp, bottom = 6.dp))
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(air.card)) {
                ui.actions.forEachIndexed { i, action ->
                    if (i > 0) RowSeparator(64.dp)
                    ActionRow(action, selected = action.key == ui.selected, enabled = action.enabled && !ui.busy) { onSelect(action.key) }
                }
            }
            Spacer(Modifier.size(18.dp))
            Text("TO", color = air.muted, fontSize = 12.sp, modifier = Modifier.padding(start = 14.dp, bottom = 6.dp))
            Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(air.card)) {
                if (ui.peers.isEmpty()) {
                    Text("Looking for devices… Open AirBridge on your computer.", color = air.muted, fontSize = 13.sp, modifier = Modifier.padding(14.dp))
                }
                ui.peers.forEachIndexed { index, peer ->
                    if (index > 0) RowSeparator(72.dp)
                    DeviceRow(peer, index, ui) { onPick(peer) }
                }
            }
        }
    }
}

@Composable
private fun ActionRow(action: EdgeAction, selected: Boolean, enabled: Boolean, onClick: () -> Unit) {
    val air = LocalAir.current
    Row(
        Modifier
            .fillMaxWidth()
            .alpha(if (action.enabled) 1f else 0.45f)
            .pressable(onClick, enabled = enabled)
            .padding(horizontal = 12.dp, vertical = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(air.fill), contentAlignment = Alignment.Center) {
            val thumbnail = action.thumbnail
            if (thumbnail != null) Image(thumbnail, null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            else Icon(action.icon, contentDescription = null, tint = air.accent, modifier = Modifier.size(21.dp))
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(action.title, color = air.text, fontSize = 15.sp)
            Text(action.subtitle, color = air.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        // The chosen item gets a check mark, like an iOS selection list.
        if (selected) Icon(Icons.Rounded.Check, contentDescription = "Selected", tint = air.accent, modifier = Modifier.size(20.dp))
    }
}

@Composable
private fun DeviceRow(peer: Peer, index: Int, ui: EdgePanelUi, onClick: () -> Unit) {
    val air = LocalAir.current
    val isTarget = peer.id == ui.targetId
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        delay(60L + 40L * index)
        appear.animateTo(1f, tween(260))
    }
    val dim by animateFloatAsState(if (ui.targetId != null && !isTarget) 0.4f else 1f, tween(250), label = "dim")
    val (mode, subtitle) = targetState(peer, ui.targetId, ui.phase, ui.trusted)
    Row(
        Modifier
            .fillMaxWidth()
            .graphicsLayer { alpha = dim * appear.value }
            .pressable(onClick, enabled = ui.selected != null && !ui.busy && ui.phase != SharePhase.DONE)
            .padding(horizontal = 10.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(52.dp), contentAlignment = Alignment.Center) {
            ProgressRing(mode, ui.progress, Modifier.fillMaxSize(), stroke = 3.dp)
            Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 42.dp)
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(peer.alias, color = air.text, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                subtitle,
                color = when (mode) {
                    RingMode.FAILED -> air.danger
                    RingMode.PROGRESS, RingMode.SUCCESS -> air.accent
                    else -> air.muted
                },
                fontSize = 12.sp,
                maxLines = 1,
            )
        }
    }
}
