package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Android
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.Dns
import androidx.compose.material.icons.rounded.FolderZip
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Language
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.Movie
import androidx.compose.material.icons.rounded.MusicNote
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.PictureAsPdf
import androidx.compose.material.icons.rounded.TabletAndroid
import androidx.compose.material.icons.rounded.Terminal
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay

// ------------------------------------------------------------- formatting

fun formatBytes(bytes: Long): String {
    if (bytes < 1000) return "$bytes B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes / 1000.0
    var unit = 0
    while (value >= 1000 && unit < units.size - 1) {
        value /= 1000
        unit++
    }
    return if (value >= 100) "%.0f %s".format(value, units[unit]) else "%.1f %s".format(value, units[unit])
}

fun timeAgo(millis: Long): String {
    val seconds = (System.currentTimeMillis() - millis) / 1000
    return when {
        seconds < 45 -> "just now"
        seconds < 3600 -> "${seconds / 60} min ago"
        seconds < 86400 -> "${seconds / 3600} h ago"
        else -> java.text.DateFormat.getDateInstance(java.text.DateFormat.SHORT).format(java.util.Date(millis))
    }
}

// ----------------------------------------------------------------- icons

fun deviceIcon(type: String?, model: String? = null): ImageVector = when (type) {
    "mobile" -> if (model?.contains("tab", true) == true || model?.contains("pad", true) == true) Icons.Rounded.TabletAndroid else Icons.Rounded.PhoneAndroid
    "web" -> Icons.Rounded.Language
    "server" -> Icons.Rounded.Dns
    "headless" -> Icons.Rounded.Terminal
    else -> Icons.Rounded.Laptop
}

fun fileIcon(mime: String, name: String): ImageVector {
    val n = name.lowercase()
    return when {
        mime.startsWith("image/") -> Icons.Rounded.Image
        mime.startsWith("video/") -> Icons.Rounded.Movie
        mime.startsWith("audio/") -> Icons.Rounded.MusicNote
        mime == "application/pdf" || n.endsWith(".pdf") -> Icons.Rounded.PictureAsPdf
        mime.contains("android.package") || n.endsWith(".apk") -> Icons.Rounded.Android
        Regex("zip|tar|rar|7z|compressed").containsMatchIn(mime) || Regex("\\.(zip|tar|gz|7z|rar|xz)$").containsMatchIn(n) -> Icons.Rounded.FolderZip
        else -> Icons.Rounded.Description
    }
}

/** A device: a gray circle with its glyph, like contacts and devices on iOS. */
@Composable
fun Avatar(icon: ImageVector, size: Dp, modifier: Modifier = Modifier) {
    val air = LocalAir.current
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(Brush.verticalGradient(listOf(air.avatarTop, air.avatarBottom))),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(size * 0.46f))
    }
}

// ---------------------------------------------------------- progress ring

enum class RingMode { IDLE, WAITING, PROGRESS, SUCCESS, FAILED }

/**
 * The ring around a device while sharing, like the iOS share sheet: a short
 * arc turns while waiting, the blue ring fills while sending, and once sent
 * it stays full for a moment and fades.
 */
@Composable
fun ProgressRing(mode: RingMode, progress: Float, modifier: Modifier = Modifier, stroke: Dp = 3.5.dp) {
    val air = LocalAir.current
    val animated by animateFloatAsState(
        when (mode) {
            RingMode.PROGRESS -> progress
            RingMode.SUCCESS -> 1f
            else -> 0f
        },
        tween(300, easing = LinearEasing),
        label = "ring",
    )
    val inspection = LocalInspectionMode.current
    val fade = remember { Animatable(1f) }
    LaunchedEffect(mode) {
        fade.snapTo(1f)
        if (mode == RingMode.SUCCESS && !inspection) {
            delay(1600)
            fade.animateTo(0f, tween(500))
        }
    }
    val spin by rememberInfiniteTransition(label = "spin").animateFloat(0f, 360f, infiniteRepeatable(tween(1400, easing = LinearEasing)), label = "a")
    Canvas(modifier) {
        if (mode == RingMode.IDLE || mode == RingMode.FAILED) return@Canvas
        val s = stroke.toPx()
        val inset = Offset(s / 2, s / 2)
        val arcSize = Size(size.width - s, size.height - s)
        val alpha = fade.value
        drawArc(air.fill, 0f, 360f, false, inset, arcSize, alpha = alpha, style = Stroke(s))
        if (mode == RingMode.WAITING) {
            drawArc(air.accent, spin - 90f, 60f, false, inset, arcSize, style = Stroke(s, cap = StrokeCap.Round))
        } else {
            drawArc(air.accent, -90f, 360f * animated, false, inset, arcSize, alpha = alpha, style = Stroke(s, cap = StrokeCap.Round))
        }
    }
}

/** This phone, visible to others: a blue mark with slow waves around it. */
@Composable
fun Beacon(on: Boolean, modifier: Modifier = Modifier, size: Dp = 56.dp) {
    val air = LocalAir.current
    val color = if (on) air.accent else air.faint
    val wave by rememberInfiniteTransition(label = "beacon").animateFloat(0f, 1f, infiniteRepeatable(tween(4000, easing = LinearEasing)), label = "w")
    Box(modifier.size(size * 2.2f), contentAlignment = Alignment.Center) {
        if (on) {
            Canvas(Modifier.fillMaxSize()) {
                val r = size.toPx() / 2
                for (i in 0 until 2) {
                    val k = (wave + i * 0.5f) % 1f
                    drawCircle(color, r * (1f + 1.2f * k), alpha = 0.32f * (1f - k), style = Stroke(1.5.dp.toPx()))
                }
            }
        }
        Box(Modifier.size(size).clip(CircleShape).background(color), contentAlignment = Alignment.Center) {
            Icon(Icons.Rounded.Wifi, contentDescription = null, tint = Color.White, modifier = Modifier.size(size * 0.52f))
        }
    }
}

@Composable
fun StatusDot(on: Boolean, modifier: Modifier = Modifier) {
    val air = LocalAir.current
    Box(modifier.size(7.dp).clip(CircleShape).background(if (on) air.success else air.faint))
}
