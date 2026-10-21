package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
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
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.sin
import kotlin.random.Random

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

/** Stable pair of colors for a device id, so every device keeps its look. */
fun avatarColors(id: String): Pair<Color, Color> {
    var h = -2128831035
    for (c in id) {
        h = h xor c.code
        h *= 16777619
    }
    val a = abs(h % 360).toFloat()
    val b = (a + 40 + abs((h shr 9) % 70)) % 360
    return Color.hsl(a, 0.85f, 0.64f) to Color.hsl(b, 0.8f, 0.52f)
}

@Composable
fun Avatar(id: String, icon: ImageVector, size: Dp, modifier: Modifier = Modifier, colors: Pair<Color, Color>? = null) {
    val (a, b) = remember(id, colors) { colors ?: avatarColors(id) }
    Box(
        modifier
            .size(size)
            .shadow(18.dp, CircleShape, ambientColor = b, spotColor = b)
            .clip(CircleShape)
            .background(Brush.linearGradient(listOf(a, b))),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, contentDescription = null, tint = Color.White, modifier = Modifier.size(size * 0.46f))
    }
}

// ---------------------------------------------------------------- aurora

@Composable
fun Aurora(modifier: Modifier = Modifier) {
    val air = LocalAir.current
    val t = rememberInfiniteTransition(label = "aurora")
    val p by t.animateFloat(0f, 1f, infiniteRepeatable(tween(26_000, easing = LinearEasing), RepeatMode.Reverse), label = "p")
    val alpha = if (air.dark) 0.42f else 0.28f
    Canvas(modifier) {
        val w = size.width
        val h = size.height
        val r = maxOf(w, h) * 0.55f
        fun blob(color: Color, cx: Float, cy: Float) =
            drawCircle(Brush.radialGradient(listOf(color.copy(alpha = alpha), Color.Transparent), Offset(cx, cy), r), r, Offset(cx, cy))
        blob(Color(0xFF5B4BFF), w * (0.15f + 0.2f * p), h * (0.12f + 0.1f * sin(p * PI.toFloat())))
        blob(Color(0xFFFF3D8F), w * (0.95f - 0.25f * p), h * (0.35f + 0.15f * p))
        blob(Color(0xFF19C3FF), w * (0.4f + 0.3f * sin(p * PI.toFloat())), h * (0.95f - 0.15f * p))
    }
}

// ---------------------------------------------------------- progress ring

enum class RingMode { IDLE, WAITING, PROGRESS, SUCCESS, FAILED }

@Composable
fun ProgressRing(mode: RingMode, progress: Float, modifier: Modifier = Modifier, stroke: Dp = 4.dp) {
    val air = LocalAir.current
    val animated by animateFloatAsState(if (mode == RingMode.PROGRESS) progress else if (mode == RingMode.IDLE) 0f else 1f, tween(350, easing = FastOutSlowInEasing), label = "ring")
    val spin = rememberInfiniteTransition(label = "spin").animateFloat(0f, 360f, infiniteRepeatable(tween(2400, easing = LinearEasing)), label = "a")
    Canvas(modifier) {
        val s = stroke.toPx()
        val inset = s / 2
        val arcSize = Size(size.width - s, size.height - s)
        if (mode == RingMode.IDLE) return@Canvas
        drawArc(air.border, 0f, 360f, false, Offset(inset, inset), arcSize, style = Stroke(s))
        when (mode) {
            RingMode.WAITING -> rotate(spin.value) {
                drawArc(
                    Brush.sweepGradient(listOf(Violet, Pink, Cyan, Violet)), 0f, 360f, false, Offset(inset, inset), arcSize,
                    style = Stroke(s, cap = StrokeCap.Round, pathEffect = PathEffect.dashPathEffect(floatArrayOf(s * 3.5f, s * 3f))),
                )
            }
            RingMode.SUCCESS -> drawArc(Mint, -90f, 360f, false, Offset(inset, inset), arcSize, style = Stroke(s))
            RingMode.FAILED -> drawArc(Rose, -90f, 360f, false, Offset(inset, inset), arcSize, style = Stroke(s))
            else -> rotate(-90f) {
                drawArc(Brush.sweepGradient(listOf(Violet, Pink, Cyan, Violet)), 0f, 360f * animated, false, Offset(inset, inset), arcSize, style = Stroke(s, cap = StrokeCap.Round))
            }
        }
    }
}

// --------------------------------------------------------------- effects

private class Particle(
    var x: Float, var y: Float, var vx: Float, var vy: Float,
    var life: Float, var age: Float, val size: Float, val color: Color,
    val confetti: Boolean, var rot: Float = 0f, val vr: Float = 0f,
)

private class StreamParticle(
    val key: String, var t: Float, val speed: Float, val bend: Float, val size: Float, val jitter: Float,
)

class Stream(val from: () -> Offset?, val to: () -> Offset?, val color: Color) {
    internal var acc = 0f
}

/** One particle system for the whole screen: confetti, sparkles and transfer streams. */
@Stable
class Fx {
    private val particles = mutableListOf<Particle>()
    private val streamParticles = mutableListOf<StreamParticle>()
    private val streams = mutableMapOf<String, Stream>()
    var active by mutableStateOf(false)
        private set
    internal var frame by mutableLongStateOf(0L)

    private fun wake() {
        active = particles.isNotEmpty() || streams.isNotEmpty() || streamParticles.isNotEmpty()
    }

    fun confetti(at: Offset, count: Int = 90) {
        val colors = listOf(Violet, Pink, Cyan, Mint, Amber, Color.White)
        repeat(count) {
            val angle = Random.nextFloat() * 2 * PI.toFloat()
            val speed = 500f + Random.nextFloat() * 1100f
            particles += Particle(at.x, at.y, cos(angle) * speed, sin(angle) * speed - 700f, 1.4f + Random.nextFloat(), 0f,
                10f + Random.nextFloat() * 12f, colors.random(), true, Random.nextFloat() * 360f, (Random.nextFloat() - 0.5f) * 720f)
        }
        wake()
    }

    fun sparkle(at: Offset, color: Color = Cyan, count: Int = 18) {
        repeat(count) { i ->
            val angle = i / count.toFloat() * 2 * PI.toFloat()
            val speed = 200f + Random.nextFloat() * 160f
            particles += Particle(at.x, at.y, cos(angle) * speed, sin(angle) * speed, 0.7f, 0f, 6f, color, false)
        }
        wake()
    }

    fun stream(key: String, from: () -> Offset?, to: () -> Offset?, color: Color) {
        if (key !in streams) streams[key] = Stream(from, to, color)
        wake()
    }

    fun stop(key: String) {
        streams.remove(key)
        wake()
    }

    fun keepOnly(keys: Set<String>) {
        streams.keys.retainAll(keys)
        wake()
    }

    internal fun step(dt: Float) {
        for ((key, s) in streams) {
            s.acc += dt * 40f
            while (s.acc > 1f) {
                s.acc -= 1f
                streamParticles += StreamParticle(key, 0f, 0.55f + Random.nextFloat() * 0.5f, 0.22f + (Random.nextFloat() - 0.5f) * 0.5f, 4f + Random.nextFloat() * 5f, (Random.nextFloat() - 0.5f) * 26f)
            }
        }
        streamParticles.removeAll { p ->
            p.t += dt * p.speed
            p.t >= 1f || p.key !in streams
        }
        particles.removeAll { p ->
            p.age += dt
            if (p.confetti) p.vy += 1700f * dt
            val drag = if (p.confetti) 0.985f else 0.95f
            p.vx *= drag
            p.vy *= drag
            p.x += p.vx * dt
            p.y += p.vy * dt
            p.rot += p.vr * dt
            p.age >= p.life
        }
        wake()
    }

    internal fun draw(scope: androidx.compose.ui.graphics.drawscope.DrawScope) = with(scope) {
        for (p in streamParticles) {
            val s = streams[p.key] ?: continue
            val a = s.from() ?: continue
            val b = s.to() ?: continue
            val mid = Offset((a.x + b.x) / 2, (a.y + b.y) / 2)
            val dx = b.x - a.x
            val dy = b.y - a.y
            val c = Offset(mid.x - dy * p.bend, mid.y + dx * p.bend)
            val t = p.t
            val u = 1 - t
            val x = u * u * a.x + 2 * u * t * c.x + t * t * b.x + p.jitter * sin(t * PI.toFloat())
            val y = u * u * a.y + 2 * u * t * c.y + t * t * b.y
            val alpha = sin(t * PI.toFloat())
            drawCircle(s.color.copy(alpha = alpha * 0.35f), p.size * 2.2f, Offset(x, y), blendMode = BlendMode.Plus)
            drawCircle(s.color.copy(alpha = alpha), p.size * 0.8f, Offset(x, y), blendMode = BlendMode.Plus)
        }
        for (p in particles) {
            val alpha = (1 - p.age / p.life).coerceIn(0f, 1f)
            if (p.confetti) {
                rotate(p.rot, Offset(p.x, p.y)) {
                    drawRect(p.color.copy(alpha = alpha), Offset(p.x - p.size / 2, p.y - p.size / 4), Size(p.size, p.size / 2))
                }
            } else {
                drawCircle(p.color.copy(alpha = alpha), p.size, Offset(p.x, p.y), blendMode = BlendMode.Plus)
            }
        }
    }
}

@Composable
fun FxLayer(fx: Fx, modifier: Modifier = Modifier) {
    LaunchedEffect(fx.active) {
        var last = 0L
        while (fx.active) {
            withFrameNanos { now ->
                val dt = if (last == 0L) 0.016f else ((now - last) / 1e9f).coerceAtMost(0.05f)
                last = now
                fx.step(dt)
                fx.frame = now
            }
        }
    }
    Canvas(modifier) {
        fx.frame // read to redraw every frame
        fx.draw(this)
    }
}

@Composable
fun PulseDot(on: Boolean, modifier: Modifier = Modifier) {
    val t = rememberInfiniteTransition(label = "dot")
    val p by t.animateFloat(0f, 1f, infiniteRepeatable(tween(2200)), label = "p")
    Canvas(modifier.size(10.dp).padding(1.dp)) {
        val color = if (on) Mint else Color.Gray
        if (on) drawCircle(color.copy(alpha = (1 - p) * 0.6f), size.minDimension / 2 * (1 + p * 1.4f))
        drawCircle(color, size.minDimension / 2)
    }
}

@Composable
fun RadarBackdrop(modifier: Modifier = Modifier, fast: Boolean = false) {
    val air = LocalAir.current
    val t = rememberInfiniteTransition(label = "radar")
    val pulse by t.animateFloat(0f, 1f, infiniteRepeatable(tween(if (fast) 1800 else 4500, easing = LinearEasing)), label = "pulse")
    val sweep by t.animateFloat(0f, 360f, infiniteRepeatable(tween(7000, easing = LinearEasing)), label = "sweep")
    val orbit by t.animateFloat(0f, 360f, infiniteRepeatable(tween(120_000, easing = LinearEasing)), label = "orbit")
    Canvas(modifier.fillMaxSize()) {
        val c = center
        val r = size.minDimension / 2
        // Dashed orbits where devices sit.
        rotate(orbit, c) {
            drawCircle(air.border.copy(alpha = 0.5f), r * 0.6f, c, style = Stroke(2f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(10f, 12f))))
        }
        rotate(-orbit * 0.7f, c) {
            drawCircle(air.border.copy(alpha = 0.3f), r * 0.92f, c, style = Stroke(2f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(6f, 14f))))
        }
        // Expanding pulses.
        for (i in 0 until 3) {
            val k = (pulse + i / 3f) % 1f
            drawCircle(Violet.copy(alpha = (1 - k) * 0.55f), r * (0.2f + k * 0.85f), c, style = Stroke(3f))
        }
        // Rotating sweep.
        rotate(sweep, c) {
            drawCircle(
                Brush.sweepGradient(0f to Color.Transparent, 0.11f to Cyan.copy(alpha = 0.16f), 0.2f to Color.Transparent, center = c),
                r * 0.95f, c,
            )
        }
    }
}
