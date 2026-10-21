package io.github.mdhaarith.airbridge.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color

val Violet = Color(0xFF7C6CFF)
val Pink = Color(0xFFFF5FA2)
val Cyan = Color(0xFF33D1FF)
val Mint = Color(0xFF34D399)
val Rose = Color(0xFFFB7185)
val Amber = Color(0xFFFBBF24)

val AccentGradient = Brush.linearGradient(listOf(Violet, Pink))

@Immutable
data class AirColors(
    val dark: Boolean,
    val bg: Color,
    val surface: Color,
    val surfaceStrong: Color,
    val solid: Color,
    val border: Color,
    val text: Color,
    val muted: Color,
    val faint: Color,
)

private val DarkAir = AirColors(
    dark = true,
    bg = Color(0xFF070914),
    surface = Color.White.copy(alpha = 0.06f),
    surfaceStrong = Color.White.copy(alpha = 0.1f),
    solid = Color(0xFF141831),
    border = Color.White.copy(alpha = 0.1f),
    text = Color(0xFFEEF0FF),
    muted = Color(0xFF9BA1C6),
    faint = Color(0xFF646A92),
)

private val LightAir = AirColors(
    dark = false,
    bg = Color(0xFFF2F3FB),
    surface = Color.White.copy(alpha = 0.7f),
    surfaceStrong = Color.White,
    solid = Color.White,
    border = Color(0x1A181C46),
    text = Color(0xFF12152B),
    muted = Color(0xFF555B80),
    faint = Color(0xFF8C91B0),
)

val LocalAir = staticCompositionLocalOf { DarkAir }

@Composable
fun AirBridgeTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val air = if (dark) DarkAir else LightAir
    val scheme = if (dark) {
        darkColorScheme(primary = Violet, secondary = Pink, tertiary = Cyan, background = air.bg, surface = air.solid, onSurface = air.text, onBackground = air.text, surfaceVariant = Color(0xFF1C2140), onSurfaceVariant = air.muted, error = Rose)
    } else {
        lightColorScheme(primary = Violet, secondary = Pink, tertiary = Cyan, background = air.bg, surface = air.solid, onSurface = air.text, onBackground = air.text, surfaceVariant = Color(0xFFE6E8F6), onSurfaceVariant = air.muted, error = Rose)
    }
    CompositionLocalProvider(LocalAir provides air) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
