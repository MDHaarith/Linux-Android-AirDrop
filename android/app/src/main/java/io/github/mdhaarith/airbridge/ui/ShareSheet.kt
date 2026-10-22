package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Link
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
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

/** The ring and the line under a device's name, the same in the share sheet and the edge panel. */
internal fun targetState(peer: Peer, targetId: String?, phase: SharePhase, trusted: Set<String>): Pair<RingMode, String> {
    val isTarget = peer.id == targetId
    return when {
        isTarget && phase == SharePhase.WAITING -> RingMode.WAITING to "Waiting…"
        isTarget && phase == SharePhase.SENDING -> RingMode.PROGRESS to "Sending…"
        isTarget && phase == SharePhase.DONE -> RingMode.SUCCESS to "Sent"
        isTarget && phase == SharePhase.FAILED -> RingMode.FAILED to "Not Sent"
        peer.id in trusted -> RingMode.IDLE to "My device"
        else -> RingMode.IDLE to (peer.deviceModel ?: if (peer.deviceType == "mobile") "Phone" else "Computer")
    }
}

/**
 * The share sheet: slides up over the app you shared from, like the iOS
 * share sheet's AirDrop row. Tap a device and it sends; the ring around it
 * shows the progress and the sheet folds away once the other side has it.
 */
@Composable
fun ShareSheetContent(
    ui: ShareSheetUi,
    onPick: (Peer) -> Unit,
    onDismiss: () -> Unit,
    onRetry: () -> Unit,
    onScan: () -> Unit,
) {
    val air = LocalAir.current
    val inspection = LocalInspectionMode.current
    val shown = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) { shown.animateTo(1f, tween(380, easing = androidx.compose.animation.core.CubicBezierEasing(0.16f, 1f, 0.3f, 1f))) }

    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier
                .fillMaxSize()
                .alpha(shown.value.coerceIn(0f, 1f))
                .background(Color.Black.copy(alpha = 0.25f))
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onDismiss),
        )
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .graphicsLayer { translationY = (1f - shown.value) * size.height }
                .clip(RoundedCornerShape(topStart = 14.dp, topEnd = 14.dp))
                .background(air.bg)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .navigationBarsPadding()
                .padding(bottom = 16.dp),
        ) {
            PreviewHeader(ui.preview, onDismiss)
            Box(Modifier.fillMaxWidth().height(0.5.dp).background(air.separator))
            Spacer(Modifier.height(16.dp))
            if (ui.peers.isEmpty() && ui.phase != SharePhase.LOOKING) {
                EmptyDevices(onScan)
            } else {
                Row(
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 10.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    if (ui.phase == SharePhase.LOOKING && ui.target == null) LookingTarget(ui.lookingFor ?: "Device")
                    ui.peers.forEachIndexed { index, peer -> DeviceTarget(peer, index, ui) { onPick(peer) } }
                }
            }
            val note = when {
                ui.phase == SharePhase.FAILED -> ui.error ?: "Couldn't send"
                ui.phase == SharePhase.LOOKING -> "Looking for ${ui.lookingFor ?: "the device"}…"
                ui.phase == SharePhase.PICK && ui.peers.isNotEmpty() -> "Tap a device to send"
                else -> null
            }
            if (note != null) {
                Text(
                    note,
                    color = if (ui.phase == SharePhase.FAILED) air.danger else air.muted,
                    fontSize = 13.sp,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 10.dp),
                )
            }
            if (ui.phase == SharePhase.FAILED) {
                Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    SecondaryButton("Close", modifier = Modifier.weight(1f), onClick = onDismiss)
                    PrimaryButton("Try Again", modifier = Modifier.weight(1f), onClick = onRetry)
                }
            }
        }
    }
}

@Composable
private fun PreviewHeader(preview: SharePreview, onClose: () -> Unit) {
    val air = LocalAir.current
    Row(Modifier.padding(start = 16.dp, end = 12.dp, top = 16.dp, bottom = 14.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(52.dp).clip(RoundedCornerShape(8.dp)).background(air.card), contentAlignment = Alignment.Center) {
            val thumbnail = preview.thumbnail
            if (thumbnail != null) {
                Image(thumbnail, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
            } else {
                val icon = when {
                    preview.isLink -> Icons.Rounded.Link
                    preview.mime == null -> Icons.Rounded.Description
                    else -> fileIcon(preview.mime, preview.title)
                }
                Icon(icon, contentDescription = null, tint = air.accent, modifier = Modifier.size(26.dp))
            }
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(preview.title, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(preview.subtitle, color = air.muted, fontSize = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Box(Modifier.size(30.dp).clip(CircleShape).background(air.fill).clickable(onClick = onClose), contentAlignment = Alignment.Center) {
            Icon(Icons.Rounded.Close, contentDescription = "Close", tint = air.muted, modifier = Modifier.size(17.dp))
        }
    }
}

@Composable
private fun DeviceTarget(peer: Peer, index: Int, ui: ShareSheetUi, onClick: () -> Unit) {
    val air = LocalAir.current
    val isTarget = peer.id == ui.targetId
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        delay(40L * index)
        appear.animateTo(1f, tween(260))
    }
    val dim by animateFloatAsState(if (ui.targetId != null && !isTarget) 0.4f else 1f, tween(250), label = "dim")
    val (mode, subtitle) = targetState(peer, ui.targetId, ui.phase, ui.trusted)
    Column(
        Modifier
            .width(84.dp)
            .graphicsLayer { alpha = dim * appear.value }
            .pressable(onClick, enabled = !ui.busy && ui.phase != SharePhase.DONE)
            .padding(vertical = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(74.dp), contentAlignment = Alignment.Center) {
            ProgressRing(mode, ui.progress, Modifier.fillMaxSize())
            Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 62.dp)
        }
        Spacer(Modifier.height(6.dp))
        Text(peer.alias, color = air.text, fontSize = 12.sp, maxLines = 2, textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis, lineHeight = 15.sp)
        Text(
            subtitle,
            color = when (mode) {
                RingMode.FAILED -> air.danger
                RingMode.PROGRESS, RingMode.SUCCESS -> air.accent
                else -> air.muted
            },
            fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun LookingTarget(name: String) {
    val air = LocalAir.current
    Column(Modifier.width(84.dp).padding(vertical = 4.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.size(74.dp), contentAlignment = Alignment.Center) {
            ProgressRing(RingMode.WAITING, 0f, Modifier.fillMaxSize())
            Box(Modifier.size(62.dp).clip(CircleShape).background(air.fill))
        }
        Spacer(Modifier.height(6.dp))
        Text(name, color = air.text, fontSize = 12.sp, maxLines = 2, textAlign = TextAlign.Center, overflow = TextOverflow.Ellipsis)
        Text("Looking…", color = air.muted, fontSize = 11.sp)
    }
}

@Composable
private fun EmptyDevices(onScan: () -> Unit) {
    val air = LocalAir.current
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp).clip(RoundedCornerShape(12.dp)).background(air.card).padding(vertical = 22.dp, horizontal = 20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("No One Nearby", color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
        Spacer(Modifier.height(3.dp))
        Text("Open AirBridge on your computer, on the same Wi‑Fi.", color = air.muted, fontSize = 13.sp, textAlign = TextAlign.Center)
        Spacer(Modifier.height(10.dp))
        Text("Look Again", color = air.accent, fontSize = 15.sp, modifier = Modifier.pressable(onScan).padding(4.dp))
    }
}
