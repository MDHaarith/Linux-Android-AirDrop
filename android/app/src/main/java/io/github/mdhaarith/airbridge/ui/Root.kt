package io.github.mdhaarith.airbridge.ui

import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Previews
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.core.sharePhrase

@Composable
fun AirBridgeRoot(vm: AppViewModel) {
    val air = LocalAir.current
    Box(Modifier.fillMaxSize().background(air.bg)) {
        AnimatedContent(
            targetState = vm.screen,
            transitionSpec = {
                // A navigation push: the new screen slides in from the right.
                val forward = targetState.ordinal > initialState.ordinal
                (slideInHorizontally(tween(320)) { if (forward) it else -it / 3 } + fadeIn(tween(120)))
                    .togetherWith(slideOutHorizontally(tween(320)) { if (forward) -it / 3 else it } + fadeOut(tween(320)))
            },
            label = "screen",
        ) { screen ->
            when (screen) {
                Screen.HOME -> HomeScreen(vm)
                Screen.ACTIVITY -> ActivityScreen(vm)
                Screen.SETTINGS -> SettingsScreen(vm)
            }
        }
        BackHandler(enabled = vm.screen != Screen.HOME) { vm.screen = Screen.HOME }
        Banners(vm, Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 6.dp))
        IncomingAlert(vm)
        PinDialog(vm)
    }
}

/** Notification banners, like iOS: a rounded card with a small colored glyph. */
@Composable
private fun Banners(vm: AppViewModel, modifier: Modifier) {
    val air = LocalAir.current
    Column(modifier.fillMaxWidth().padding(horizontal = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (toast in vm.toasts.take(3)) {
            key(toast.id) {
                var shown by remember { mutableStateOf(false) }
                LaunchedEffect(Unit) { shown = true }
                AnimatedVisibility(shown, enter = slideInVertically(tween(350)) { -it } + fadeIn(tween(200))) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .shadow(16.dp, RoundedCornerShape(20.dp))
                            .clip(RoundedCornerShape(20.dp))
                            .background(air.elevated)
                            .padding(13.dp),
                        verticalAlignment = Alignment.Top,
                    ) {
                        val (icon, tint) = when {
                            toast.copyText != null -> Icons.Rounded.ChatBubble to air.accent
                            toast.good -> Icons.Rounded.Check to air.success
                            else -> Icons.Rounded.Close to air.danger
                        }
                        Box(Modifier.size(30.dp).clip(RoundedCornerShape(8.dp)).background(tint), contentAlignment = Alignment.Center) {
                            Icon(icon, null, tint = Color.White, modifier = Modifier.size(18.dp))
                        }
                        Spacer(Modifier.width(11.dp))
                        Column(Modifier.weight(1f)) {
                            Text(toast.title, color = air.text, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
                            if (toast.text.isNotEmpty()) Text(toast.text, color = air.muted, fontSize = 14.sp, maxLines = 6, overflow = TextOverflow.Ellipsis)
                            if (toast.copyText != null) {
                                Spacer(Modifier.height(8.dp))
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    var copied by remember { mutableStateOf(false) }
                                    SecondaryButton(if (copied) "Copied" else "Copy", height = 34.dp) {
                                        vm.engine.copyToClipboard(toast.copyText)
                                        copied = true
                                    }
                                    SecondaryButton("Dismiss", height = 34.dp) { vm.toasts.remove(toast) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun IncomingAlert(vm: AppViewModel) {
    val transfers by vm.engine.transfers.collectAsStateWithLifecycle()
    val waiting = vm.incoming.firstNotNullOfOrNull { id ->
        transfers.firstOrNull { it.id == id && it.direction == Direction.IN && it.status == Status.WAITING }
    }
    LaunchedEffect(transfers) {
        vm.incoming.removeAll { id -> transfers.none { it.id == id && it.status == Status.WAITING } }
    }
    val t = waiting ?: return
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(t.id) { haptics.performHapticFeedback(HapticFeedbackType.LongPress) }
    // A request needs an answer: tapping outside does nothing, Back keeps it in Activity.
    Dialog(
        onDismissRequest = { vm.dismissIncoming(t.id) },
        properties = DialogProperties(dismissOnClickOutside = false, usePlatformDefaultWidth = false),
    ) {
        IncomingContent(
            t,
            onDecline = {
                vm.engine.decide(t.id, accept = false)
                vm.dismissIncoming(t.id)
            },
            onAccept = { trust ->
                vm.engine.decide(t.id, accept = true, trust = trust)
                vm.dismissIncoming(t.id)
            },
        )
    }
}

/** An incoming request, like AirDrop's alert: what is coming, who sends it, Decline / Accept. */
@Composable
fun IncomingContent(t: Transfer, onDecline: () -> Unit, onAccept: (trust: Boolean) -> Unit) {
    val air = LocalAir.current
    var trust by remember(t.id) { mutableStateOf(false) }
    val photos = remember(t.id) {
        t.files.asSequence().mapNotNull { Previews.decode(it.preview) }.take(Previews.MAX_PREVIEWS)
            .mapNotNull { BitmapFactory.decodeByteArray(it, 0, it.size)?.asImageBitmap() }.toList()
    }
    AlertCard(
        listOf(
            AlertAction("Decline") { onDecline() },
            AlertAction("Accept", bold = true) { onAccept(trust) },
        ),
    ) {
        IncomingPreview(t, photos)
        Spacer(Modifier.height(14.dp))
        Text("AirBridge", color = air.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
        Spacer(Modifier.height(2.dp))
        Text(
            "“${t.peerAlias}” would like to share ${sharePhrase(t.files)}.",
            color = air.text, fontSize = 13.sp, lineHeight = 17.sp, textAlign = TextAlign.Center,
        )
        val names = if (t.files.size > 1) t.files.take(2).joinToString(", ") { it.name.substringAfterLast('/') } + if (t.files.size > 2) " and ${t.files.size - 2} more" else "" else null
        Text(
            listOfNotNull(names, formatBytes(t.total)).joinToString(" · "),
            color = air.muted, fontSize = 12.sp, textAlign = TextAlign.Center, maxLines = 2, overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 3.dp),
        )
        // Quick Share-style senders have no lasting identity to trust.
        if (t.peerModel != "Quick Share") {
            Spacer(Modifier.height(12.dp))
            Row(Modifier.pressable({ trust = !trust }).padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                Box(
                    Modifier.size(18.dp).clip(CircleShape)
                        .background(if (trust) air.accent else Color.Transparent)
                        .border(1.5.dp, if (trust) air.accent else air.faint, CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    if (trust) Icon(Icons.Rounded.Check, null, tint = Color.White, modifier = Modifier.size(13.dp))
                }
                Spacer(Modifier.width(7.dp))
                Text("Always accept from this device", color = air.muted, fontSize = 13.sp)
            }
        }
    }
}

@Composable
private fun IncomingPreview(t: Transfer, photos: List<ImageBitmap>) {
    val air = LocalAir.current
    when {
        photos.size == 1 && t.files.size == 1 -> {
            val photo = photos.first()
            Image(
                photo, contentDescription = t.files.first().name, contentScale = ContentScale.Crop,
                modifier = Modifier
                    .widthIn(max = 258.dp)
                    .heightIn(max = 210.dp)
                    .aspectRatio(photo.width.toFloat() / photo.height, matchHeightConstraintsFirst = photo.height > photo.width)
                    .clip(RoundedCornerShape(10.dp)),
            )
        }
        photos.isNotEmpty() -> Box(Modifier.size(170.dp), contentAlignment = Alignment.Center) {
            // A small stack, the first photo on top.
            val angles = listOf(0f, 5f, -7f)
            photos.withIndex().reversed().forEach { (i, photo) ->
                Image(
                    photo, contentDescription = null, contentScale = ContentScale.Crop,
                    modifier = Modifier
                        .size(146.dp)
                        .rotate(angles[i])
                        .shadow(6.dp, RoundedCornerShape(10.dp))
                        .clip(RoundedCornerShape(10.dp))
                        .border(3.dp, Color.White, RoundedCornerShape(10.dp)),
                )
            }
        }
        else -> {
            val first = t.files.firstOrNull()
            val mixed = t.files.map { it.mime }.distinct().size > 1
            Box(Modifier.size(68.dp).clip(RoundedCornerShape(15.dp)).background(air.fill), contentAlignment = Alignment.Center) {
                Icon(
                    if (first == null || mixed) fileIcon("", "") else fileIcon(first.mime, first.name),
                    null, tint = air.accent, modifier = Modifier.size(32.dp),
                )
            }
        }
    }
}

/** An alert with a text field, like iOS prompts (rename, PIN, a quick message). */
@Composable
fun TextAlert(
    title: String,
    message: String?,
    initial: String,
    confirm: String,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
    placeholder: String = "",
    singleLine: Boolean = true,
    allowEmpty: Boolean = false,
    keyboardType: KeyboardType = KeyboardType.Text,
    filter: (String) -> String = { it },
    extra: @Composable (set: (String) -> Unit) -> Unit = {},
) {
    val air = LocalAir.current
    var value by remember { mutableStateOf(initial) }
    val focus = remember { FocusRequester() }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        AlertCard(
            listOf(
                AlertAction("Cancel") { onDismiss() },
                AlertAction(confirm, bold = true) { if (value.isNotBlank() || allowEmpty) onConfirm(value) },
            ),
        ) {
            Text(title, color = air.text, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center)
            if (message != null) Text(message, color = air.text, fontSize = 13.sp, textAlign = TextAlign.Center, modifier = Modifier.padding(top = 2.dp))
            Spacer(Modifier.height(12.dp))
            BasicTextField(
                value, { value = filter(it) },
                singleLine = singleLine,
                textStyle = TextStyle(color = air.text, fontSize = 15.sp),
                cursorBrush = SolidColor(air.accent),
                keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = if (singleLine) 34.dp else 110.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(air.card)
                    .border(0.5.dp, air.separator, RoundedCornerShape(8.dp))
                    .padding(horizontal = 9.dp, vertical = 8.dp)
                    .focusRequester(focus),
                decorationBox = { field ->
                    Box {
                        if (value.isEmpty()) Text(placeholder, color = air.faint, fontSize = 15.sp)
                        field()
                    }
                },
            )
            extra { value = filter(it) }
        }
    }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
}

@Composable
private fun PinDialog(vm: AppViewModel) {
    val peer = vm.pinFor ?: return
    TextAlert(
        title = "PIN for ${peer.alias}",
        message = "This device only accepts files with a PIN. Enter it, then send again.",
        initial = vm.engine.pins[peer.id].orEmpty(),
        confirm = "Save",
        keyboardType = KeyboardType.Number,
        filter = { it.filter(Char::isLetterOrDigit).take(32) },
        onConfirm = { pin ->
            vm.engine.pins[peer.id] = pin
            vm.pinFor = null
            vm.toast("PIN saved", "Send your files again")
        },
        onDismiss = { vm.pinFor = null },
    )
}
