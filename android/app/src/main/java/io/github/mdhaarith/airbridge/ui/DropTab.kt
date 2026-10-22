package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Download
import androidx.compose.material.icons.rounded.Groups
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** A place to drop on while dragging: one device, or everyone nearby. */
data class DropTarget(
    val id: String,
    val name: String,
    val subtitle: String?,
    val deviceType: String? = null,
    val deviceModel: String? = null,
    val everyone: Boolean = false,
)

data class DropTabUi(
    val rightSide: Boolean,
    val targets: List<DropTarget>,
    /** The target under the finger, if any. */
    val hovered: String? = null,
    /** Something is being dragged over the tab. */
    val over: Boolean = false,
    val defaultId: String? = null,
)

/**
 * The tab's geometry, in dp: the window is sized from it and the drag
 * position is mapped back to a row with it, so both always agree.
 */
object DropTabLayout {
    const val HEADER = 30
    const val ROW = 68
    const val FOOTER = 8
    const val WIDTH = 196
    const val COMPACT_WIDTH = 104
    const val COMPACT_HEIGHT = 168

    fun width(targets: Int) = if (targets == 0) COMPACT_WIDTH else WIDTH
    fun height(targets: Int) = if (targets == 0) COMPACT_HEIGHT else HEADER + targets * ROW + FOOTER

    /** Index of the row at [yDp] from the top of the tab, or null over the header. */
    fun rowAt(yDp: Float, targets: Int): Int? =
        if (yDp < HEADER) null else ((yDp - HEADER) / ROW).toInt().takeIf { it in 0 until targets }
}

/**
 * What the edge handle becomes while something is dragged anywhere on the
 * screen: a card against the edge listing nearby devices. The row under the
 * finger lights up blue, like a device in the AirDrop sheet.
 */
@Composable
fun DropTab(ui: DropTabUi) {
    val air = LocalAir.current
    // Flat against the screen edge, rounded on the inside.
    val shape = if (ui.rightSide) RoundedCornerShape(topStart = 22.dp, bottomStart = 22.dp) else RoundedCornerShape(topEnd = 22.dp, bottomEnd = 22.dp)
    val edge = if (ui.rightSide) Modifier.padding(start = 4.dp) else Modifier.padding(end = 4.dp)
    if (ui.targets.isEmpty()) return CompactTab(ui, shape, edge)

    Column(
        edge.fillMaxSize()
            .clip(shape)
            .background(air.elevated)
            .border(0.5.dp, air.separator, shape)
            .padding(horizontal = 6.dp),
    ) {
        Text(
            "DROP TO SEND",
            color = air.muted, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 0.6.sp,
            modifier = Modifier.height(DropTabLayout.HEADER.dp).padding(start = 10.dp, top = 12.dp),
        )
        for (target in ui.targets) TargetRow(target, hovered = target.id == ui.hovered, isDefault = target.id == ui.defaultId)
    }
}

@Composable
private fun TargetRow(target: DropTarget, hovered: Boolean, isDefault: Boolean) {
    val air = LocalAir.current
    val fill by animateColorAsState(if (hovered) air.accent else Color.Transparent, tween(140), label = "fill")
    val scale by animateFloatAsState(if (hovered) 1.08f else 1f, tween(160), label = "scale")
    Row(
        Modifier.fillMaxWidth().height(DropTabLayout.ROW.dp).padding(vertical = 3.dp)
            .clip(RoundedCornerShape(16.dp)).background(fill).padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.graphicsLayer { scaleX = scale; scaleY = scale }) {
            if (target.everyone) {
                Box(
                    Modifier.size(46.dp).clip(CircleShape).background(if (hovered) Color.White.copy(alpha = 0.22f) else air.accent.copy(alpha = 0.14f)),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Rounded.Groups, null, tint = if (hovered) Color.White else air.accent, modifier = Modifier.size(24.dp))
                }
            } else {
                Avatar(deviceIcon(target.deviceType, target.deviceModel), 46.dp, if (hovered) Modifier.border(2.dp, Color.White, CircleShape) else Modifier)
            }
        }
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            Text(
                target.name, color = if (hovered) Color.White else air.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
            val subtitle = if (isDefault) "Default" else target.subtitle
            if (subtitle != null) {
                Text(
                    subtitle,
                    color = when {
                        hovered -> Color.White.copy(alpha = 0.85f)
                        isDefault -> air.accent
                        else -> air.muted
                    },
                    fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

/** No devices to choose from: a small "Drop here" tab that turns blue under the finger. */
@Composable
private fun CompactTab(ui: DropTabUi, shape: RoundedCornerShape, edge: Modifier) {
    val air = LocalAir.current
    val fill by animateColorAsState(if (ui.over) air.accent else air.elevated, tween(140), label = "fill")
    val ink = if (ui.over) Color.White else air.accent
    Column(
        edge.fillMaxSize().clip(shape).background(fill).border(0.5.dp, air.separator, shape).padding(10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
    ) {
        Box(Modifier.size(42.dp).clip(CircleShape).background(if (ui.over) Color.White.copy(alpha = 0.22f) else air.accent.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) {
            Icon(Icons.Rounded.Download, null, tint = ink, modifier = Modifier.size(22.dp))
        }
        Spacer(Modifier.height(10.dp))
        Text(
            if (ui.over) "Release" else "Drop here",
            color = if (ui.over) Color.White else air.text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
        )
        Text(
            if (ui.over) "to send" else "No devices yet",
            color = if (ui.over) Color.White.copy(alpha = 0.85f) else air.muted, fontSize = 11.sp, textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 2.dp),
        )
    }
}
