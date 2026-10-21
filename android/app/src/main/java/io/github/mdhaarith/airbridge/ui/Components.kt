package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Scales down while pressed, springs back: every tappable thing gets this. */
@Composable
fun Modifier.pressable(onClick: () -> Unit, enabled: Boolean = true): Modifier {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.94f else 1f, spring(0.45f, 600f), label = "press")
    return this
        .graphicsLayer { scaleX = scale; scaleY = scale }
        .clickable(source, indication = null, enabled = enabled, onClick = onClick)
}

@Composable
fun GradientButton(text: String, icon: ImageVector?, modifier: Modifier = Modifier, height: Dp = 54.dp, onClick: () -> Unit) {
    Row(
        modifier
            .height(height)
            .pressable(onClick)
            .clip(RoundedCornerShape(20.dp))
            .background(AccentGradient)
            .padding(horizontal = 18.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = Color.White, modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(8.dp))
        }
        Text(text, color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = 16.sp, maxLines = 1)
    }
}

@Composable
fun GhostButton(text: String, icon: ImageVector?, modifier: Modifier = Modifier, height: Dp = 54.dp, onClick: () -> Unit) {
    val air = LocalAir.current
    Row(
        modifier
            .height(height)
            .pressable(onClick)
            .clip(RoundedCornerShape(20.dp))
            .background(air.surface)
            .border(1.dp, air.border, RoundedCornerShape(20.dp))
            .padding(horizontal = 14.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = air.text, modifier = Modifier.size(19.dp))
            Spacer(Modifier.width(7.dp))
        }
        Text(text, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
fun GlassCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val air = LocalAir.current
    Column(
        modifier
            .clip(RoundedCornerShape(20.dp))
            .background(if (air.dark) air.solid.copy(alpha = 0.85f) else Color.White.copy(alpha = 0.92f))
            .border(BorderStroke(1.dp, air.border), RoundedCornerShape(20.dp))
            .padding(14.dp),
        content = content,
    )
}

@Composable
fun Pill(text: String, color: Color) {
    Text(
        text,
        color = color,
        fontSize = 12.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier
            .clip(RoundedCornerShape(50))
            .background(color.copy(alpha = 0.14f))
            .padding(horizontal = 9.dp, vertical = 2.dp),
    )
}

@Composable
fun SectionTitle(text: String) {
    Text(
        text.uppercase(),
        color = LocalAir.current.faint,
        style = MaterialTheme.typography.labelMedium,
        fontWeight = FontWeight.Bold,
        letterSpacing = 1.2.sp,
        modifier = Modifier.fillMaxWidth().padding(start = 4.dp, top = 22.dp, bottom = 8.dp),
    )
}

@Composable
fun IconCircleButton(icon: ImageVector, description: String, modifier: Modifier = Modifier, badge: Int = 0, onClick: () -> Unit) {
    val air = LocalAir.current
    Box(modifier.size(44.dp).pressable(onClick), contentAlignment = Alignment.Center) {
        Box(
            Modifier
                .size(42.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(air.surface)
                .border(1.dp, air.border, RoundedCornerShape(14.dp)),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, description, tint = air.text, modifier = Modifier.size(21.dp))
        }
        if (badge > 0) {
            Text(
                "$badge",
                color = Color.White,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .clip(RoundedCornerShape(50))
                    .background(Pink)
                    .padding(horizontal = 6.dp, vertical = 1.dp),
            )
        }
    }
}
