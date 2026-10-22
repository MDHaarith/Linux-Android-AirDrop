package io.github.mdhaarith.airbridge.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBackIos
import androidx.compose.material.icons.automirrored.rounded.KeyboardArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Dims slightly while pressed, like iOS controls (no ripple). */
@Composable
fun Modifier.pressable(onClick: () -> Unit, enabled: Boolean = true): Modifier {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val alpha by animateFloatAsState(if (pressed) 0.55f else 1f, tween(if (pressed) 0 else 220), label = "press")
    return this
        .graphicsLayer { this.alpha = alpha }
        .clickable(source, indication = null, enabled = enabled, onClick = onClick)
}

/** The filled blue button. */
@Composable
fun PrimaryButton(text: String, icon: ImageVector? = null, modifier: Modifier = Modifier, height: Dp = 50.dp, onClick: () -> Unit) {
    val air = LocalAir.current
    Row(
        modifier
            .height(height)
            .clip(RoundedCornerShape(12.dp))
            .background(air.accent)
            .pressable(onClick)
            .padding(horizontal = 18.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = Color.White, modifier = Modifier.size(19.dp))
            Spacer(Modifier.width(7.dp))
        }
        Text(text, color = Color.White, fontWeight = FontWeight.SemiBold, fontSize = 17.sp, maxLines = 1)
    }
}

/** The gray button with blue text. */
@Composable
fun SecondaryButton(text: String, icon: ImageVector? = null, modifier: Modifier = Modifier, height: Dp = 50.dp, onClick: () -> Unit) {
    val air = LocalAir.current
    Row(
        modifier
            .height(height)
            .clip(RoundedCornerShape(12.dp))
            .background(air.fill)
            .pressable(onClick)
            .padding(horizontal = 14.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Icon(icon, null, tint = air.accent, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
        }
        Text(text, color = air.accent, fontWeight = FontWeight.SemiBold, fontSize = if (height < 40.dp) 14.sp else 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** A tile with a blue glyph over a short label, like the action buttons on an iOS contact card. */
@Composable
fun ActionTile(text: String, icon: ImageVector, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val air = LocalAir.current
    Column(
        modifier
            .clip(RoundedCornerShape(12.dp))
            .background(air.card)
            .pressable(onClick)
            .padding(vertical = 10.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, tint = air.accent, modifier = Modifier.size(24.dp))
        Spacer(Modifier.height(4.dp))
        Text(text, color = air.accent, fontSize = 12.sp, fontWeight = FontWeight.Medium, maxLines = 1)
    }
}

/** A white (or dark gray) rounded card. */
@Composable
fun Card(modifier: Modifier = Modifier, padding: Dp = 14.dp, content: @Composable ColumnScope.() -> Unit) {
    val air = LocalAir.current
    Column(modifier.clip(RoundedCornerShape(12.dp)).background(air.card).padding(padding), content = content)
}

/** An iOS inset grouped section: a small header, rows in a card, an optional footer. */
@Composable
fun Group(title: String? = null, footer: String? = null, modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val air = LocalAir.current
    Column(modifier.fillMaxWidth().padding(bottom = 22.dp)) {
        if (title != null) {
            Text(title.uppercase(), color = air.muted, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, bottom = 7.dp))
        }
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(air.card), content = content)
        if (footer != null) {
            Text(footer, color = air.muted, fontSize = 13.sp, modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 7.dp))
        }
    }
}

/** A thin line between rows, inset to where the text starts. */
@Composable
fun RowSeparator(start: Dp = 16.dp) {
    val air = LocalAir.current
    Box(Modifier.fillMaxWidth().padding(start = start).height(0.5.dp).background(air.separator))
}

/** A row of a grouped list: an optional colored icon square, a title (and subtitle), and trailing content. */
@Composable
fun ListRow(
    title: String,
    subtitle: String? = null,
    icon: ImageVector? = null,
    iconColor: Color? = null,
    onClick: (() -> Unit)? = null,
    chevron: Boolean = onClick != null,
    titleColor: Color? = null,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    val air = LocalAir.current
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .then(if (onClick != null) Modifier.pressable(onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (icon != null) {
            Box(Modifier.size(29.dp).clip(RoundedCornerShape(7.dp)).background(iconColor ?: air.accent), contentAlignment = Alignment.Center) {
                Icon(icon, null, tint = Color.White, modifier = Modifier.size(18.dp))
            }
            Spacer(Modifier.width(14.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(title, color = titleColor ?: air.text, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (subtitle != null) Text(subtitle, color = air.muted, fontSize = 13.sp, lineHeight = 17.sp)
        }
        trailing()
        if (chevron) {
            Spacer(Modifier.width(6.dp))
            Icon(Icons.AutoMirrored.Rounded.KeyboardArrowRight, null, tint = air.faint, modifier = Modifier.size(22.dp))
        }
    }
}

/** The iOS switch: green when on, a white knob that slides. */
@Composable
fun AppleSwitch(checked: Boolean, onChange: (Boolean) -> Unit, modifier: Modifier = Modifier) {
    val air = LocalAir.current
    val track by animateColorAsState(if (checked) air.success else air.fill, tween(200), label = "track")
    val knob by animateDpAsState(if (checked) 20.dp else 0.dp, spring(0.75f, 600f), label = "knob")
    Box(
        modifier
            .size(51.dp, 31.dp)
            .clip(CircleShape)
            .background(track)
            .clickable(remember { MutableInteractionSource() }, indication = null) { onChange(!checked) }
            .padding(2.dp),
    ) {
        Box(Modifier.offset(x = knob).size(27.dp).shadow(2.dp, CircleShape).clip(CircleShape).background(Color.White))
    }
}

@Composable
fun Pill(text: String, color: Color) {
    Text(
        text,
        color = color,
        fontSize = 11.sp,
        fontWeight = FontWeight.SemiBold,
        modifier = Modifier
            .clip(RoundedCornerShape(50))
            .background(color.copy(alpha = 0.14f))
            .padding(horizontal = 7.dp, vertical = 1.dp),
    )
}

/** A plain blue glyph in a navigation bar, with an optional red badge. */
@Composable
fun BarButton(icon: ImageVector, description: String, modifier: Modifier = Modifier, badge: Int = 0, onClick: () -> Unit) {
    val air = LocalAir.current
    Box(modifier.size(44.dp).pressable(onClick), contentAlignment = Alignment.Center) {
        Icon(icon, description, tint = air.accent, modifier = Modifier.size(24.dp))
        if (badge > 0) {
            Text(
                "$badge",
                color = Color.White,
                fontSize = 11.sp,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(top = 4.dp, end = 2.dp)
                    .widthIn(min = 17.dp)
                    .clip(CircleShape)
                    .background(air.danger)
                    .padding(horizontal = 4.dp),
            )
        }
    }
}

/** A pushed screen's bar: "‹ Back" on the left, the title in the middle, an action on the right. */
@Composable
fun NavBar(title: String, back: String, onBack: () -> Unit, action: @Composable () -> Unit = {}) {
    val air = LocalAir.current
    Box(Modifier.fillMaxWidth().height(48.dp)) {
        Row(Modifier.align(Alignment.CenterStart).pressable(onBack).padding(horizontal = 8.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Rounded.ArrowBackIos, "Back", tint = air.accent, modifier = Modifier.size(20.dp))
            Text(back, color = air.accent, fontSize = 17.sp)
        }
        Text(title, color = air.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.align(Alignment.Center))
        Box(Modifier.align(Alignment.CenterEnd).padding(end = 8.dp)) { action() }
    }
}

/** A text button in a bar. */
@Composable
fun BarTextButton(text: String, onClick: () -> Unit) {
    Text(text, color = LocalAir.current.accent, fontSize = 17.sp, modifier = Modifier.pressable(onClick).padding(8.dp))
}

/** One button of an alert's bottom row. */
data class AlertAction(val label: String, val bold: Boolean = false, val destructive: Boolean = false, val onClick: () -> Unit)

/** The iOS alert: a centered card, the content, then a row of plain buttons. */
@Composable
fun AlertCard(actions: List<AlertAction>, modifier: Modifier = Modifier, width: Dp = 290.dp, content: @Composable ColumnScope.() -> Unit) {
    val air = LocalAir.current
    Column(modifier.width(width).shadow(24.dp, RoundedCornerShape(14.dp)).clip(RoundedCornerShape(14.dp)).background(air.elevated)) {
        Column(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 18.dp, bottom = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            content = content,
        )
        Box(Modifier.fillMaxWidth().height(0.5.dp).background(air.separator))
        Row(Modifier.fillMaxWidth().height(46.dp)) {
            actions.forEachIndexed { i, action ->
                if (i > 0) Box(Modifier.width(0.5.dp).height(46.dp).background(air.separator))
                Box(Modifier.weight(1f).height(46.dp).pressable(action.onClick), contentAlignment = Alignment.Center) {
                    Text(
                        action.label,
                        color = if (action.destructive) air.danger else air.accent,
                        fontSize = 17.sp,
                        fontWeight = if (action.bold) FontWeight.SemiBold else FontWeight.Normal,
                    )
                }
            }
        }
    }
}
