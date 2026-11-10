@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.mdhaarith.airbridge.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
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
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Groups
import androidx.compose.material.icons.rounded.RadioButtonUnchecked
import androidx.compose.material3.*
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.mdhaarith.airbridge.R
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.overlay.DropTargets
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
    val defaultTarget: String? = null,
    val hoverTarget: String? = null,
) {
    val busy
        get() = phase == SharePhase.WAITING || phase == SharePhase.SENDING
}

/**
 * The panel that slides out of the screen edge over any app: pick what to send (clipboard, latest
 * photo, something you dropped), then a device.
 */
@Composable
fun EdgePanelContent(
    ui: EdgePanelUi,
    onSelect: (String) -> Unit,
    onPick: (Peer) -> Unit,
    onClose: () -> Unit,
    onDefault: (String?) -> Unit = {},
    onSendAll: () -> Unit = {},
    onDropBounds: (String, Rect) -> Unit = { _, _ -> },
) {
    val colors = MaterialTheme.colorScheme
    val inspection = LocalInspectionMode.current
    val shown = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        shown.animateTo(1f, tween(360, easing = CubicBezierEasing(0.16f, 1f, 0.3f, 1f)))
    }
    val sign = if (ui.rightSide) 1f else -1f

    Box(Modifier.fillMaxSize()) {
        Box(
            Modifier.fillMaxSize()
                .alpha(shown.value.coerceIn(0f, 1f))
                .background(colors.scrim.copy(alpha = 0.32f))
                .clickable(
                    remember { MutableInteractionSource() },
                    indication = null,
                    onClick = onClose,
                )
        )
        Column(
            Modifier.align(if (ui.rightSide) Alignment.CenterEnd else Alignment.CenterStart)
                .padding(vertical = 40.dp, horizontal = 10.dp)
                .width(296.dp)
                .heightIn(max = 720.dp)
                .graphicsLayer { translationX = sign * (1f - shown.value) * (size.width + 60f) }
                .shadow(8.dp, MaterialTheme.shapes.extraLarge)
                .clip(MaterialTheme.shapes.extraLarge)
                .background(colors.surfaceContainer)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .verticalScroll(rememberScrollState())
                .padding(vertical = 16.dp)
        ) {
            Row(
                Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Image(
                    painterResource(R.drawable.ic_logo),
                    contentDescription = null,
                    modifier = Modifier.size(28.dp),
                )
                Spacer(Modifier.width(10.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        "AirBridge",
                        color = colors.onSurface,
                        fontWeight = FontWeight.SemiBold,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Text(
                        when (ui.phase) {
                            SharePhase.WAITING -> "Waiting for the other side…"
                            SharePhase.SENDING -> "Sending…"
                            SharePhase.DONE -> "Sent"
                            SharePhase.FAILED -> ui.error ?: "Couldn't send"
                            else -> "Drop on a device bubble to send"
                        },
                        color =
                            when (ui.phase) {
                                SharePhase.FAILED -> colors.error
                                SharePhase.DONE,
                                SharePhase.SENDING -> colors.primary
                                else -> colors.onSurfaceVariant
                            },
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                IconButton(onClick = onClose) {
                    Icon(
                        Icons.Rounded.Close,
                        contentDescription = "Close",
                        tint = colors.onSurfaceVariant,
                        modifier = Modifier.size(16.dp),
                    )
                }
            }
            Text(
                "Send",
                color = colors.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(start = 14.dp, bottom = 6.dp),
            )
            Column(
                Modifier.fillMaxWidth()
                    .clip(MaterialTheme.shapes.medium)
                    .background(colors.surfaceContainerLow)
            ) {
                ui.actions.forEachIndexed { i, action ->
                    if (i > 0) HorizontalDivider(modifier = Modifier.padding(start = 64.dp))
                    ActionRow(
                        action,
                        selected = action.key == ui.selected,
                        enabled = action.enabled && !ui.busy,
                    ) {
                        onSelect(action.key)
                    }
                }
            }
            Spacer(Modifier.size(18.dp))
            Text(
                "To",
                color = colors.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(start = 14.dp, bottom = 6.dp),
            )
            Text(
                "Choose a default for future drops.",
                color = colors.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
                modifier = Modifier.padding(start = 14.dp, bottom = 8.dp),
            )
            Column(
                Modifier.fillMaxWidth()
                    .clip(MaterialTheme.shapes.medium)
                    .background(colors.surfaceContainerLow)
            ) {
                if (ui.peers.isEmpty()) {
                    Text(
                        "Looking for devices… Open AirBridge on your computer.",
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(16.dp),
                    )
                }
                ui.peers.forEachIndexed { index, peer ->
                    if (index > 0) HorizontalDivider(modifier = Modifier.padding(start = 72.dp))
                    DeviceRow(
                        peer,
                        index,
                        ui,
                        onDefault = {
                            onDefault(if (ui.defaultTarget == peer.id) null else peer.id)
                        },
                        onBounds = { onDropBounds(peer.id, it) },
                    ) {
                        onPick(peer)
                    }
                }
                if (ui.peers.isNotEmpty()) {
                    HorizontalDivider(modifier = Modifier.padding(start = 72.dp))
                    Row(
                        Modifier.fillMaxWidth()
                            .onGloballyPositioned {
                                onDropBounds(DropTargets.ALL, it.boundsInRoot())
                            }
                            .background(
                                if (ui.hoverTarget == DropTargets.ALL)
                                    colors.surfaceContainerHighest
                                else Color.Transparent
                            )
                            .clickable(onClick = onSendAll, enabled = !ui.busy)
                            .padding(horizontal = 10.dp, vertical = 7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Box(Modifier.size(52.dp), contentAlignment = Alignment.Center) {
                            Icon(
                                Icons.Rounded.Groups,
                                contentDescription = null,
                                tint = colors.primary,
                                modifier =
                                    Modifier.size(42.dp)
                                        .clip(CircleShape)
                                        .background(colors.surfaceContainerHighest)
                                        .padding(8.dp),
                            )
                        }
                        Column(Modifier.weight(1f).padding(start = 10.dp)) {
                            Text(
                                "Send to all",
                                color = colors.onSurface,
                                style = MaterialTheme.typography.bodyLarge,
                            )
                            Text(
                                "${ui.peers.size} nearby devices",
                                color = colors.onSurfaceVariant,
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        DefaultButton(
                            ui.defaultTarget == DropTargets.ALL,
                            !ui.busy,
                            "all devices",
                        ) {
                            onDefault(
                                if (ui.defaultTarget == DropTargets.ALL) null else DropTargets.ALL
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ActionRow(
    action: EdgeAction,
    selected: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    ListItem(
        modifier =
            Modifier.alpha(if (enabled) 1f else 0.38f)
                .selectable(
                    selected = selected,
                    enabled = enabled,
                    role = Role.RadioButton,
                    onClick = onClick,
                ),
        headlineContent = { Text(action.title) },
        supportingContent = {
            Text(action.subtitle, maxLines = 2, overflow = TextOverflow.Ellipsis)
        },
        leadingContent = {
            val thumbnail = action.thumbnail
            if (thumbnail != null)
                Image(
                    thumbnail,
                    null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.size(40.dp).clip(MaterialTheme.shapes.small),
                )
            else Icon(action.icon, null, tint = colors.primary)
        },
        trailingContent = { RadioButton(selected = selected, onClick = null, enabled = enabled) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
    )
}

@Composable
private fun DeviceRow(
    peer: Peer,
    index: Int,
    ui: EdgePanelUi,
    onDefault: () -> Unit,
    onBounds: (Rect) -> Unit,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val isTarget = peer.id == ui.targetId || ui.targetId == DropTargets.ALL
    val inspection = LocalInspectionMode.current
    val appear = remember { Animatable(if (inspection) 1f else 0f) }
    LaunchedEffect(Unit) {
        delay(60L + 40L * index)
        appear.animateTo(1f, tween(260))
    }
    val dim by
        animateFloatAsState(
            if (ui.targetId != null && !isTarget) 0.4f else 1f,
            tween(250),
            label = "dim",
        )
    val (mode, subtitle) =
        targetState(
            peer,
            if (ui.targetId == DropTargets.ALL) peer.id else ui.targetId,
            ui.phase,
            ui.trusted,
        )
    ListItem(
        modifier =
            Modifier.onGloballyPositioned { onBounds(it.boundsInRoot()) }
                .graphicsLayer { alpha = dim * appear.value }
                .clickable(onClick = onClick, enabled = !ui.busy && ui.phase != SharePhase.DONE),
        headlineContent = { Text(peer.alias, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = {
            Text(
                subtitle,
                color =
                    when (mode) {
                        RingMode.FAILED -> colors.error
                        RingMode.PROGRESS,
                        RingMode.SUCCESS -> colors.primary
                        else -> colors.onSurfaceVariant
                    },
                maxLines = 2,
            )
        },
        leadingContent = {
            Box(Modifier.size(48.dp), contentAlignment = Alignment.Center) {
                ProgressRing(mode, ui.progress, Modifier.fillMaxSize(), stroke = 3.dp)
                Avatar(deviceIcon(peer.deviceType, peer.deviceModel), 40.dp)
            }
        },
        trailingContent = {
            DefaultButton(ui.defaultTarget == peer.id, !ui.busy, peer.alias, onDefault)
        },
        colors =
            ListItemDefaults.colors(
                containerColor =
                    if (ui.hoverTarget == peer.id) colors.secondaryContainer else Color.Transparent
            ),
    )
}

@Composable
private fun DefaultButton(selected: Boolean, enabled: Boolean, name: String, onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    IconButton(onClick = onClick, enabled = enabled) {
        Icon(
            if (selected) Icons.Rounded.CheckCircle else Icons.Rounded.RadioButtonUnchecked,
            contentDescription = if (selected) "Clear default: $name" else "Use $name for drops",
            tint = if (selected) colors.primary else colors.onSurfaceVariant.copy(alpha = 0.5f),
            modifier =
                Modifier.size(24.dp)
                    .clip(CircleShape)
                    .background(if (selected) colors.surfaceContainerHighest else Color.Transparent),
        )
    }
}
