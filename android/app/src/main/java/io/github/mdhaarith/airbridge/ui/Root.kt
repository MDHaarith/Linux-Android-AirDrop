@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)

package io.github.mdhaarith.airbridge.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.*
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.PreviewImages
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Previews
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.core.sharePhrase

@Composable
fun AirBridgeRoot(vm: AppViewModel) {
    val colors = MaterialTheme.colorScheme
    Box(Modifier.fillMaxSize()) {
        AnimatedContent(
            targetState = vm.screen,
            modifier = Modifier.fillMaxSize().background(colors.surface),
            transitionSpec = {
                (fadeIn(tween(220)) + slideInVertically(tween(220)) { it / 16 }).togetherWith(
                    fadeOut(tween(90))
                )
            },
            label = "screen",
        ) { screen ->
            when (screen) {
                Screen.HOME -> HomeScreen(vm)
                Screen.ACTIVITY -> ActivityScreen(vm)
                Screen.SETTINGS -> SettingsScreen(vm)
                Screen.COMPANION -> CompanionScreen(vm)
                Screen.SCAN -> ScanScreen(vm)
            }
        }
        BackHandler(enabled = vm.screen != Screen.HOME) { vm.screen = Screen.HOME }
        Banners(vm, Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 8.dp))
        IncomingAlert(vm)
        PinDialog(vm)
    }
}

/** Material banners retain the existing stacked toast and clipboard actions. */
@Composable
private fun Banners(vm: AppViewModel, modifier: Modifier) {
    Column(
        modifier.fillMaxWidth().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        for (toast in vm.toasts.take(3)) {
            key(toast.id) {
                ElevatedCard(Modifier.fillMaxWidth()) {
                    ListItem(
                        headlineContent = { Text(toast.title) },
                        supportingContent = {
                            if (toast.text.isNotEmpty())
                                Text(toast.text, maxLines = 6, overflow = TextOverflow.Ellipsis)
                        },
                        leadingContent = {
                            Icon(
                                when {
                                    toast.copyText != null -> Icons.Rounded.ChatBubble
                                    toast.good -> Icons.Rounded.Check
                                    else -> Icons.Rounded.Close
                                },
                                null,
                                tint =
                                    if (toast.good) MaterialTheme.colorScheme.primary
                                    else MaterialTheme.colorScheme.error,
                            )
                        },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    )
                    if (toast.copyText != null) {
                        Row(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                            var copied by remember { mutableStateOf(false) }
                            TextButton(
                                onClick = {
                                    vm.engine.copyToClipboard(toast.copyText)
                                    copied = true
                                }
                            ) {
                                Text(if (copied) "Copied" else "Copy")
                            }
                            TextButton(onClick = { vm.toasts.remove(toast) }) { Text("Dismiss") }
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
    val waiting =
        vm.incoming.firstNotNullOfOrNull { id ->
            transfers.firstOrNull {
                it.id == id && it.direction == Direction.IN && it.status == Status.WAITING
            }
        }
    LaunchedEffect(transfers) {
        vm.incoming.removeAll { id ->
            transfers.none { it.id == id && it.status == Status.WAITING }
        }
    }
    val t = waiting ?: return
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(t.id) { haptics.performHapticFeedback(HapticFeedbackType.LongPress) }
    // A request needs an answer: tapping outside does nothing, Back keeps it in Activity.
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
        onDismiss = { vm.dismissIncoming(t.id) },
    )
}

/** A receive request with a local photo preview and explicit trust control. */
@Composable
fun IncomingContent(
    t: Transfer,
    onDecline: () -> Unit,
    onAccept: (trust: Boolean) -> Unit,
    onDismiss: () -> Unit = {},
) {
    var trust by remember(t.id) { mutableStateOf(false) }
    val photos =
        remember(t.id) {
            t.files
                .asSequence()
                .mapNotNull { PreviewImages.decode(it.preview) }
                .take(Previews.MAX_PREVIEWS)
                .map { it.asImageBitmap() }
                .toList()
        }
    AlertDialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(dismissOnClickOutside = false),
        title = { Text("Receive from ${t.peerAlias}?") },
        text = {
            Column(
                Modifier.verticalScroll(rememberScrollState()),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                IncomingPreview(t, photos)
                Spacer(Modifier.height(16.dp))
                Text(
                    "“${t.peerAlias}” would like to share ${sharePhrase(t.files)}.",
                    modifier = Modifier.fillMaxWidth(),
                )
                val names =
                    if (t.files.size > 1)
                        t.files.take(2).joinToString(", ") { it.name.substringAfterLast('/') } +
                            if (t.files.size > 2) " and ${t.files.size - 2} more" else ""
                    else null
                Text(
                    listOfNotNull(names, formatBytes(t.total)).joinToString(" · "),
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                if (t.peerModel != "Quick Share") {
                    Row(
                        Modifier.fillMaxWidth()
                            .toggleable(
                                value = trust,
                                role = Role.Checkbox,
                                onValueChange = { trust = it },
                            )
                            .padding(top = 8.dp)
                            .heightIn(min = 48.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(checked = trust, onCheckedChange = null)
                        Spacer(Modifier.width(12.dp))
                        Text(
                            "Always accept from this device",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onAccept(trust) }) { Text("Accept") } },
        dismissButton = { TextButton(onClick = onDecline) { Text("Decline") } },
    )
}

@Composable
private fun IncomingPreview(t: Transfer, photos: List<ImageBitmap>) {
    val colors = MaterialTheme.colorScheme
    when {
        photos.size == 1 && t.files.size == 1 -> {
            val photo = photos.first()
            Image(
                photo,
                contentDescription = t.files.first().name,
                contentScale = ContentScale.Crop,
                modifier =
                    Modifier.widthIn(max = 258.dp)
                        .heightIn(max = 210.dp)
                        .aspectRatio(
                            photo.width.toFloat() / photo.height,
                            matchHeightConstraintsFirst = photo.height > photo.width,
                        )
                        .clip(RoundedCornerShape(10.dp)),
            )
        }
        photos.isNotEmpty() ->
            Box(Modifier.size(170.dp), contentAlignment = Alignment.Center) {
                // A small stack, the first photo on top.
                val angles = listOf(0f, 5f, -7f)
                photos.withIndex().reversed().forEach { (i, photo) ->
                    Image(
                        photo,
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier =
                            Modifier.size(146.dp)
                                .rotate(angles[i])
                                .shadow(6.dp, RoundedCornerShape(10.dp))
                                .clip(RoundedCornerShape(10.dp))
                                .border(
                                    3.dp,
                                    colors.surfaceContainerHigh,
                                    RoundedCornerShape(10.dp),
                                ),
                    )
                }
            }
        else -> {
            val first = t.files.firstOrNull()
            val mixed = t.files.map { it.mime }.distinct().size > 1
            Box(
                Modifier.size(68.dp)
                    .clip(RoundedCornerShape(15.dp))
                    .background(colors.surfaceContainerHighest),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    if (first == null || mixed) fileIcon("", "")
                    else fileIcon(first.mime, first.name),
                    null,
                    tint = colors.primary,
                    modifier = Modifier.size(32.dp),
                )
            }
        }
    }
}

/** A Material input dialog shared by rename, PIN, and message actions. */
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
    var value by remember { mutableStateOf(initial) }
    val focus = remember { FocusRequester() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                if (message != null) Text(message)
                Spacer(Modifier.height(16.dp))
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = filter(it) },
                    singleLine = singleLine,
                    placeholder = { Text(placeholder) },
                    keyboardOptions = KeyboardOptions(keyboardType = keyboardType),
                    modifier =
                        Modifier.fillMaxWidth()
                            .heightIn(min = if (singleLine) 56.dp else 120.dp)
                            .focusRequester(focus),
                )
                extra { value = filter(it) }
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(value) }, enabled = value.isNotBlank() || allowEmpty) {
                Text(confirm)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
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
