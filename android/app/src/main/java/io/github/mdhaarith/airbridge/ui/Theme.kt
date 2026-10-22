package io.github.mdhaarith.airbridge.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * An Apple-like palette: grouped gray backgrounds, white (or dark gray)
 * cards, one blue accent and gray device avatars. The values follow the iOS
 * system colors so the phone matches the Linux app and the browser page.
 */
@Immutable
data class AirColors(
    val dark: Boolean,
    /** The grouped background behind cards. */
    val bg: Color,
    /** Cards, lists and sheets. */
    val card: Color,
    /** Alerts and menus floating above everything. */
    val elevated: Color,
    /** Gray fill for secondary buttons, chips and tracks. */
    val fill: Color,
    val separator: Color,
    val text: Color,
    val muted: Color,
    val faint: Color,
    val accent: Color,
    val success: Color,
    val danger: Color,
    val warning: Color,
    val avatarTop: Color,
    val avatarBottom: Color,
)

val LightAir = AirColors(
    dark = false,
    bg = Color(0xFFF2F2F7),
    card = Color.White,
    elevated = Color(0xFFF9F9F9),
    fill = Color(0x1F787880),
    separator = Color(0x4A3C3C43),
    text = Color.Black,
    muted = Color(0x993C3C43),
    faint = Color(0x4D3C3C43),
    accent = Color(0xFF007AFF),
    success = Color(0xFF34C759),
    danger = Color(0xFFFF3B30),
    warning = Color(0xFFFF9500),
    avatarTop = Color(0xFFB7BBC4),
    avatarBottom = Color(0xFF8B9099),
)

val DarkAir = AirColors(
    dark = true,
    bg = Color.Black,
    card = Color(0xFF1C1C1E),
    elevated = Color(0xFF2C2C2E),
    fill = Color(0x3D767680),
    separator = Color(0x99545458),
    text = Color.White,
    muted = Color(0x99EBEBF5),
    faint = Color(0x4DEBEBF5),
    accent = Color(0xFF0A84FF),
    success = Color(0xFF30D158),
    danger = Color(0xFFFF453A),
    warning = Color(0xFFFF9F0A),
    avatarTop = Color(0xFF7D7D83),
    avatarBottom = Color(0xFF55555A),
)

val LocalAir = staticCompositionLocalOf { LightAir }

@Composable
fun AirBridgeTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    val air = if (dark) DarkAir else LightAir
    val scheme = if (dark) {
        darkColorScheme(
            primary = air.accent, secondary = air.accent, tertiary = air.accent, background = air.bg, surface = air.card,
            onSurface = air.text, onBackground = air.text, surfaceVariant = air.elevated, onSurfaceVariant = air.muted, error = air.danger,
            surfaceContainerHigh = air.elevated, outline = air.separator,
        )
    } else {
        lightColorScheme(
            primary = air.accent, secondary = air.accent, tertiary = air.accent, background = air.bg, surface = air.card,
            onSurface = air.text, onBackground = air.text, surfaceVariant = air.elevated, onSurfaceVariant = air.muted, error = air.danger,
            surfaceContainerHigh = air.elevated, outline = air.separator,
        )
    }
    CompositionLocalProvider(LocalAir provides air) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
