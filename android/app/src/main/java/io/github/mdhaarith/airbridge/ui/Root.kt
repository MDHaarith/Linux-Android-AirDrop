package io.github.mdhaarith.airbridge.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ChatBubble
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer

@Composable
fun AirBridgeRoot(vm: AppViewModel) {
    val air = LocalAir.current
    val fx = remember { Fx() }
    Box(Modifier.fillMaxSize().background(air.bg)) {
        Aurora(Modifier.fillMaxSize())
        AnimatedContent(
            targetState = vm.screen,
            transitionSpec = {
                val forward = targetState.ordinal > initialState.ordinal
                (slideInHorizontally(spring(0.85f, 300f)) { if (forward) it / 3 else -it / 3 } + fadeIn(tween(220)))
                    .togetherWith(slideOutHorizontally(spring(0.85f, 300f)) { if (forward) -it / 4 else it / 4 } + fadeOut(tween(160)))
            },
            label = "screen",
        ) { screen ->
            when (screen) {
                Screen.HOME -> HomeScreen(vm, fx)
                Screen.ACTIVITY -> ActivityScreen(vm)
                Screen.SETTINGS -> SettingsScreen(vm)
            }
        }
        BackHandler(enabled = vm.screen != Screen.HOME) { vm.screen = Screen.HOME }
        ToastStack(vm, Modifier.align(Alignment.TopCenter).statusBarsPadding().padding(top = 8.dp))
        IncomingSheet(vm)
        PinDialog(vm)
    }
}

@Composable
private fun ToastStack(vm: AppViewModel, modifier: Modifier) {
    val air = LocalAir.current
    Column(modifier.fillMaxWidth().padding(horizontal = 14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (toast in vm.toasts.take(3)) {
            androidx.compose.runtime.key(toast.id) {
                var shown by remember { mutableStateOf(false) }
                LaunchedEffect(Unit) { shown = true }
                AnimatedVisibility(shown, enter = slideInVertically(spring(0.6f, 400f)) { -it } + fadeIn()) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(18.dp))
                            .background(air.solid)
                            .padding(14.dp),
                        verticalAlignment = Alignment.Top,
                    ) {
                        val icon = when {
                            toast.copyText != null -> Icons.Rounded.ChatBubble
                            toast.good -> Icons.Rounded.CheckCircle
                            else -> Icons.Rounded.ErrorOutline
                        }
                        Icon(icon, null, tint = if (toast.good) (if (toast.copyText != null) Cyan else Mint) else Rose, modifier = Modifier.size(22.dp))
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(toast.title, color = air.text, fontWeight = FontWeight.SemiBold)
                            if (toast.text.isNotEmpty()) Text(toast.text, color = air.muted, fontSize = 13.sp, maxLines = 6, overflow = TextOverflow.Ellipsis)
                            if (toast.copyText != null) {
                                Spacer(Modifier.height(8.dp))
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    var copied by remember { mutableStateOf(false) }
                                    GradientButton(if (copied) "Copied ✓" else "Copy", null, height = 36.dp) {
                                        vm.engine.copyToClipboard(toast.copyText)
                                        copied = true
                                    }
                                    GhostButton("Dismiss", null, height = 36.dp) { vm.toasts.remove(toast) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun IncomingSheet(vm: AppViewModel) {
    val air = LocalAir.current
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
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(onDismissRequest = { vm.dismissIncoming(t.id) }, sheetState = sheet, containerColor = air.solid) {
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

/** The body of the incoming-request sheet (separate so it can be rendered on its own). */
@Composable
fun IncomingContent(t: Transfer, onDecline: () -> Unit, onAccept: (trust: Boolean) -> Unit) {
    val air = LocalAir.current
    var trust by remember(t.id) { mutableStateOf(false) }
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 22.dp).navigationBarsPadding().padding(bottom = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(120.dp), contentAlignment = Alignment.Center) {
            val ripple = rememberInfiniteTransition(label = "ripple")
            val p by ripple.animateFloat(0f, 1f, infiniteRepeatable(tween(2200, easing = LinearEasing)), label = "p")
            Canvas(Modifier.size(120.dp)) {
                for (i in 0 until 2) {
                    val k = (p + i * 0.5f) % 1f
                    drawCircle(Cyan.copy(alpha = (1 - k) * 0.7f), size.minDimension / 2 * (0.66f + 0.34f * k), style = Stroke(4f))
                }
            }
            Avatar(t.peerId, deviceIcon(t.peerType, t.peerModel), 80.dp)
        }
        Spacer(Modifier.height(8.dp))
        val what = if (t.files.size == 1) "a file" else "${t.files.size} files"
        Text("${t.peerAlias} wants to share $what", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold, color = air.text, textAlign = TextAlign.Center)
        Text(listOfNotNull(t.peerModel, formatBytes(t.total)).joinToString(" · "), color = air.muted, modifier = Modifier.padding(top = 4.dp, bottom = 14.dp))
        LazyColumn(Modifier.fillMaxWidth().heightIn(max = 220.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            itemsIndexed(t.files.take(100)) { _, f ->
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(air.surfaceStrong).padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(fileIcon(f.mime, f.name), null, tint = Cyan, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(10.dp))
                    Text(f.name, color = air.text, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    Text(formatBytes(f.size), color = air.faint, fontSize = 12.sp)
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(vertical = 8.dp).pressable({ trust = !trust }), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
            Checkbox(trust, { trust = it }, colors = CheckboxDefaults.colors(checkedColor = Violet))
            Text("Always accept from this device", color = air.muted, fontSize = 14.sp)
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            GhostButton("Decline", null, Modifier.weight(1f)) { onDecline() }
            GradientButton("Accept", null, Modifier.weight(1f)) { onAccept(trust) }
        }
    }
}

@Composable
private fun PinDialog(vm: AppViewModel) {
    val peer = vm.pinFor ?: return
    var pin by remember(peer.id) { mutableStateOf(vm.engine.pins[peer.id].orEmpty()) }
    AlertDialog(
        onDismissRequest = { vm.pinFor = null },
        icon = { Icon(Icons.Rounded.Lock, null) },
        title = { Text("PIN for ${peer.alias}") },
        text = {
            Column {
                Text("This device only accepts files with a PIN. Enter it, then send again.")
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(pin, { pin = it.filter(Char::isLetterOrDigit).take(32) }, singleLine = true, label = { Text("PIN") })
            }
        },
        confirmButton = {
            TextButton({
                vm.engine.pins[peer.id] = pin
                vm.pinFor = null
                vm.toast("PIN saved", "Send your files again")
            }) { Text("Save") }
        },
        dismissButton = { TextButton({ vm.pinFor = null }) { Text("Cancel") } },
    )
}
