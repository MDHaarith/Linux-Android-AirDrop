package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.offset
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin
import kotlin.math.sqrt

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
    val style: DropStyle = DropStyle.LABELS,
)

/** How the devices open around the handle while something is dragged. */
enum class DropStyle { ICONS, LABELS, ROW }

/**
 * Where everything sits, in dp: the window is sized from it and the drag
 * position is mapped back to a device with it, so both always agree.
 * Spot 0 is the hub on the edge; the rest open inward around it.
 */
object DropTabLayout {
    const val HUB = 60
    const val BUBBLE = 52
    const val PILL_W = 184
    const val PILL_H = 48
    /** The pills' inner ends sit on a circle whose center is this far out past the hub. */
    const val ARC_OUT = 120f
    const val ARC_R = ARC_OUT + HUB / 2 + 24
    /** The hub's center, in from the screen edge (a little of it tucks off screen). */
    private const val EDGE = 26
    private const val PAD = 16
    /** Room above and below for the hovered device's name (icons and row only). */
    private const val CAPTION = 40

    /** How many devices (hub included) each style has room for. */
    fun capacity(style: DropStyle) = when (style) {
        DropStyle.ICONS -> 9
        DropStyle.LABELS -> 6
        DropStyle.ROW -> 5
    }

    /** The hub first (the default device, else the first), then everyone else in order. */
    fun arrange(targets: List<DropTarget>, defaultId: String?): List<DropTarget> {
        val hub = targets.firstOrNull { it.id == defaultId } ?: targets.firstOrNull { !it.everyone } ?: return emptyList()
        return listOf(hub) + targets.filter { it !== hub }
    }

    /** Where a pill [dy] from the hub ends (its side toward the hub), relative to the hub. */
    fun anchorX(dy: Float) = ARC_OUT - sqrt(ARC_R * ARC_R - dy * dy)

    class Spots(val width: Float, val height: Float, val centers: List<Offset>)

    /** Centers of [count] spots (at least the hub) in a window against the [rightSide] or left edge. */
    fun spots(style: DropStyle, count: Int, rightSide: Boolean): Spots {
        val others = (count - 1).coerceAtLeast(0)
        val hubR = HUB / 2f
        // Relative to the hub, with inward as -x.
        val around = (0 until others).map { k ->
            when (style) {
                DropStyle.ICONS -> {
                    val inner = k < 5
                    val n = if (inner) minOf(others, 5) else others - 5
                    val i = if (inner) k else k - 5
                    val radius = if (inner) 112f else 176f
                    val step = Math.toRadians(if (inner) 40.0 else 26.0)
                    val angle = (i - (n - 1) / 2f) * step
                    Offset((-radius * cos(angle)).toFloat(), (radius * sin(angle)).toFloat())
                }
                DropStyle.LABELS -> {
                    // Pills stacked so their inner ends follow an arc around the hub.
                    val dy = (k - (others - 1) / 2f) * 60f
                    Offset(anchorX(dy) - PILL_W / 2f, dy)
                }
                DropStyle.ROW -> Offset(-(hubR + 12 + BUBBLE / 2f + k * (BUBBLE + 8)), 0f)
            }
        }
        val reachX = if (style == DropStyle.LABELS) PILL_W / 2f else BUBBLE / 2f
        val reachY = if (style == DropStyle.LABELS) PILL_H / 2f else BUBBLE / 2f
        val inward = maxOf(-(around.minOfOrNull { it.x } ?: 0f) + reachX, hubR)
        val half = maxOf((around.maxOfOrNull { kotlin.math.abs(it.y) } ?: 0f) + reachY, hubR) + PAD +
            if (style == DropStyle.LABELS) 0 else CAPTION
        val width = maxOf(inward + PAD + EDGE, if (style == DropStyle.LABELS) 0f else 150f)
        val height = half * 2
        val hubX = width - EDGE
        val centers = (listOf(Offset.Zero) + around).map { Offset(hubX + it.x, height / 2 + it.y) }
        return Spots(width, height, if (rightSide) centers else centers.map { Offset(width - it.x, it.y) })
    }

    /** The spot nearest the finger at ([x], [y]) dp: anywhere on the window counts, so it is easy to hit. */
    fun hit(style: DropStyle, count: Int, rightSide: Boolean, x: Float, y: Float): Int? =
        if (count == 0) null else spots(style, count, rightSide).centers.withIndex().minByOrNull { hypot(it.value.x - x, it.value.y - y) }?.index
}

/**
 * What the edge handle becomes while something is dragged anywhere on the
 * screen: a glowing hub on the edge (your default device) with the other
 * devices opening around it as glass bubbles. The one under the finger
 * swells and turns blue, like a device in the AirDrop sheet.
 */
@Composable
fun DropTab(ui: DropTabUi) {
    val air = LocalAir.current
    val targets = DropTabLayout.arrange(ui.targets, ui.defaultId)
    val count = maxOf(targets.size, 1)
    val spots = DropTabLayout.spots(ui.style, count, ui.rightSide)
    val swell = List(count) { i ->
        val on = targets.getOrNull(i)?.let { it.id == ui.hovered } ?: ui.over
        animateFloatAsState(if (on) 1f else 0f, spring(dampingRatio = 0.5f, stiffness = 600f), label = "swell").value
    }
    val hub = spots.centers[0]

    Box(Modifier.fillMaxSize()) {
        Canvas(Modifier.fillMaxSize()) {
            val center = Offset(hub.x.dp.toPx(), hub.y.dp.toPx())
            // The hub's glow, brighter while something hovers it.
            val glow = (DropTabLayout.HUB * (0.95f + 0.25f * swell[0])).dp.toPx()
            drawCircle(Brush.radialGradient(listOf(air.accent.copy(alpha = 0.55f), Color.Transparent), center, glow), glow, center)
            // A faint track the devices sit along, with a blue bead between neighbours.
            val thread = Color.White.copy(alpha = 0.22f)
            val bead = air.accent.copy(alpha = 0.9f)
            val inward = if (ui.rightSide) 180f else 0f
            fun arc(pivot: Offset, radius: Float, points: List<Offset>) {
                if (points.isEmpty()) return
                val angles = points.map { p ->
                    val a = Math.toDegrees(atan2((p.y - pivot.y).toDouble(), (p.x - pivot.x).toDouble())).toFloat()
                    ((a - inward + 540f) % 360f) - 180f
                }.sorted()
                val pad = if (angles.size > 1) (angles[1] - angles[0]) / 2 else 20f
                drawArc(
                    thread, inward + angles.first() - pad, angles.last() - angles.first() + pad * 2, false,
                    Offset(pivot.x - radius, pivot.y - radius), Size(radius * 2, radius * 2), style = Stroke(1.dp.toPx()),
                )
                for (j in 1 until angles.size) {
                    val mid = Math.toRadians((inward + (angles[j - 1] + angles[j]) / 2).toDouble())
                    drawCircle(bead, 2.5.dp.toPx(), Offset(pivot.x + radius * cos(mid).toFloat(), pivot.y + radius * sin(mid).toFloat()))
                }
            }
            val around = spots.centers.drop(1).map { Offset(it.x.dp.toPx(), it.y.dp.toPx()) }
            when (ui.style) {
                // Each ring of bubbles on its own circle round the hub.
                DropStyle.ICONS -> around.groupBy { (it - center).getDistance().toInt() / 20 }.values
                    .forEach { ring -> arc(center, (ring[0] - center).getDistance(), ring) }
                DropStyle.LABELS -> {
                    val out = (if (ui.rightSide) 1 else -1) * DropTabLayout.ARC_OUT.dp.toPx()
                    val pivot = Offset(center.x + out, center.y)
                    val radius = (DropTabLayout.ARC_R - 14).dp.toPx()
                    arc(pivot, radius, around.map { Offset(it.x, it.y) })
                }
                DropStyle.ROW -> (listOf(center) + around).zipWithNext { a, b -> drawCircle(bead, 2.5.dp.toPx(), (a + b) / 2f) }
            }
        }

        targets.forEachIndexed { i, target ->
            if (i == 0) return@forEachIndexed
            val c = spots.centers[i]
            if (ui.style == DropStyle.LABELS) {
                // Sized to the name, its inner end on the arc; it slides toward you when picked.
                val end = if (ui.rightSide) c.x + DropTabLayout.PILL_W / 2f else c.x - DropTabLayout.PILL_W / 2f
                Box(
                    Modifier
                        .layout { measurable, constraints ->
                            val p = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0))
                            layout(constraints.maxWidth, constraints.maxHeight) {
                                val x = end.dp.roundToPx() - if (ui.rightSide) p.width else 0
                                p.place(x, c.y.dp.roundToPx() - p.height / 2)
                            }
                        }
                        .graphicsLayer { translationX = (if (ui.rightSide) -8 else 8) * swell[i] * density },
                ) { Pill(target, swell[i] > 0.5f) }
            } else {
                Spot(c, DropTabLayout.BUBBLE.dp, DropTabLayout.BUBBLE.dp, swell[i]) { Bubble(target, swell[i] > 0.5f) }
            }
        }
        Spot(hub, DropTabLayout.HUB.dp, DropTabLayout.HUB.dp, swell[0]) {
            Box(
                Modifier.fillMaxSize().clip(CircleShape)
                    .background(Brush.verticalGradient(listOf(air.accent.copy(alpha = 0.85f).compositeOver(Color.White), air.accent)))
                    .border(1.dp, Color.White.copy(alpha = 0.45f), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(targets.firstOrNull()?.let(::iconFor) ?: Icons.Rounded.Download, null, tint = Color.White, modifier = Modifier.size(28.dp))
            }
        }

        if (ui.style != DropStyle.LABELS) {
            val hovered = targets.firstOrNull { it.id == ui.hovered }
            val caption = when {
                targets.isEmpty() -> if (ui.over) "Release to send" else "No devices yet"
                hovered != null -> hovered.name
                else -> null
            }
            if (caption != null) {
                // Under the bubbles, centered on them.
                val xs = spots.centers.map { it.x }
                val bottom = spots.centers.maxOf { it.y } + (if (count > 1) DropTabLayout.BUBBLE else DropTabLayout.HUB) / 2f + 10
                Caption(caption, Offset((xs.min() + xs.max()) / 2, bottom), hovered != null || ui.over)
            }
        }
    }
}

/** Places [content] centered on [center] (dp), scaled up by [swell]. */
@Composable
private fun Spot(center: Offset, width: Dp, height: Dp, swell: Float, content: @Composable () -> Unit) {
    Box(
        Modifier.offset(center.x.dp - width / 2, center.y.dp - height / 2).size(width, height)
            .graphicsLayer { val s = 1f + 0.14f * swell; scaleX = s; scaleY = s },
    ) { content() }
}

private fun iconFor(target: DropTarget): ImageVector =
    if (target.everyone) Icons.Rounded.Groups else deviceIcon(target.deviceType, target.deviceModel)

/**
 * Glass, like the system's own panels: a smoky see-through tint with a sheen
 * across the top and a bright edge, over the frosted screen behind (see
 * EdgePanel). The same in light and dark, so white ink always reads on it;
 * under the finger it fills with the accent.
 */
@Composable
private fun Modifier.glass(on: Boolean, shape: Shape): Modifier {
    val air = LocalAir.current
    val tint = if (on) listOf(air.accent.copy(alpha = 0.95f), air.accent.copy(alpha = 0.80f)) else listOf(GLASS_TOP, GLASS_BOTTOM)
    return background(Brush.verticalGradient(tint), shape)
        .background(Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.16f), Color.Transparent), endY = 60f), shape)
        .border(1.dp, Brush.verticalGradient(listOf(Color.White.copy(alpha = 0.55f), Color.White.copy(alpha = 0.10f))), shape)
}

private val GLASS_TOP = Color(0xB83A3A3E)
private val GLASS_BOTTOM = Color(0xD11C1C1F)

@Composable
private fun Bubble(target: DropTarget, on: Boolean) {
    val air = LocalAir.current
    Box(Modifier.fillMaxSize().clip(CircleShape).glass(on, CircleShape), contentAlignment = Alignment.Center) {
        Icon(iconFor(target), null, tint = Color.White, modifier = Modifier.size(24.dp))
    }
}

@Composable
private fun Pill(target: DropTarget, on: Boolean) {
    val air = LocalAir.current
    val shape = RoundedCornerShape(50)
    Row(
        Modifier.height(DropTabLayout.PILL_H.dp).widthIn(max = DropTabLayout.PILL_W.dp).clip(shape).glass(on, shape).padding(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            Modifier.size(40.dp).clip(CircleShape).background(Color.White.copy(alpha = if (on) 0.24f else 0.12f))
                .border(0.5.dp, Color.White.copy(alpha = 0.2f), CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(iconFor(target), null, tint = Color.White, modifier = Modifier.size(22.dp))
        }
        Spacer(Modifier.width(12.dp))
        Text(
            target.name, color = Color.White, fontSize = 15.sp, fontWeight = FontWeight.Medium,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(end = 14.dp),
        )
    }
}

/** The name of the device under the finger, in a small glass capsule centered at [at]'s x, its top at [at]'s y. */
@Composable
private fun Caption(text: String, at: Offset, on: Boolean) {
    val air = LocalAir.current
    Box(
        Modifier
            .layout { measurable, constraints ->
                val p = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0))
                layout(constraints.maxWidth, constraints.maxHeight) {
                    p.place((at.x.dp.roundToPx() - p.width / 2).coerceIn(0, maxOf(constraints.maxWidth - p.width, 0)), at.y.dp.roundToPx())
                }
            }
            .clip(RoundedCornerShape(50))
            .glass(on, RoundedCornerShape(50))
            .padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        Text(text, color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
