package io.github.mdhaarith.airbridge.ui

import android.graphics.drawable.GradientDrawable
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CornerBasedShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogWindowProvider
import com.kyant.backdrop.backdrops.LayerBackdrop
import com.kyant.backdrop.backdrops.emptyBackdrop
import com.kyant.backdrop.drawBackdrop
import com.kyant.backdrop.effects.blur
import com.kyant.backdrop.effects.lens
import com.kyant.backdrop.effects.vibrancy

/** The app content that glass surfaces sample from; null where nothing records it. */
val LocalBackdrop = staticCompositionLocalOf<LayerBackdrop?> { null }

/**
 * True when the compositor already blurs what is behind this window (dialogs, the edge panel).
 * Other windows and apps can't be sampled, so glass there is the system blur plus the rim and tint.
 */
val LocalWindowBlur = staticCompositionLocalOf { false }

/** System blur radius, in dp, for windows floating over other content. */
const val WINDOW_BLUR_DP = 30

/** Backdrop blur needs RenderEffect on a RenderNode (Android 12+). */
fun isGlassSupported(sdkInt: Int = Build.VERSION.SDK_INT): Boolean = sdkInt >= Build.VERSION_CODES.S

/**
 * Liquid glass, like iOS: the content behind is saturated, blurred and refracted at the edges
 * (lens needs Android 13), with a specular rim, a drop shadow and a light or dark tint. Falls back
 * to an opaque [fallback] card where glass can't render. [shape] is corner-based because the lens
 * effect throws for any other shape.
 */
@Composable
fun Modifier.liquidGlass(shape: CornerBasedShape, fallback: Color = LocalAir.current.elevated, elevation: Dp = 16.dp): Modifier {
    val air = LocalAir.current
    val backdrop = LocalBackdrop.current?.takeIf { isGlassSupported() }
        ?: emptyBackdrop().takeIf { LocalWindowBlur.current }
        ?: return shadow(elevation, shape).clip(shape).background(fallback)
    val px = with(LocalDensity.current) { 1.dp.toPx() }
    val tint = if (air.dark) Color(0xFF121212) else Color(0xFFFAFAFA)
    return drawBackdrop(
        backdrop = backdrop,
        shape = { shape },
        effects = {
            vibrancy()
            blur(8 * px)
            lens(refractionHeight = 24 * px, refractionAmount = 24 * px, depthEffect = true, chromaticAberration = true)
        },
        onDrawSurface = { drawRect(tint.copy(alpha = 0.4f)) },
    ).clip(shape)
}

/**
 * Blurs what is behind the enclosing dialog window and returns whether that blur is on (Android 12+,
 * and off in battery saver or when the device disables cross-window blur). The dialog window wraps
 * its card, so the blur is clipped to the card's rounded rect. Returns false outside a dialog.
 */
@Composable
fun dialogWindowBlur(cornerRadius: Dp): Boolean {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return false
    val window = (LocalView.current.parent as? DialogWindowProvider)?.window ?: return false
    // shortcut: read once, so toggling battery saver mid-dialog keeps the old look until reopened.
    val enabled = remember(window) { window.windowManager.isCrossWindowBlurEnabled }
    val density = LocalDensity.current
    DisposableEffect(window) {
        // The blur region takes its corners from the window background's outline.
        window.setBackgroundDrawable(GradientDrawable().apply { this.cornerRadius = with(density) { cornerRadius.toPx() } })
        window.setBackgroundBlurRadius(with(density) { WINDOW_BLUR_DP.dp.roundToPx() })
        onDispose {}
    }
    return enabled
}
