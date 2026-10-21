package io.github.mdhaarith.airbridge.ui

import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Size
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.DeleteSweep
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.FileStatus
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.TFile
import io.github.mdhaarith.airbridge.core.Transfer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
fun ActivityScreen(vm: AppViewModel) {
    val air = LocalAir.current
    val transfers by vm.engine.transfers.collectAsStateWithLifecycle()
    val list = transfers.reversed()
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            IconCircleButton(Icons.AutoMirrored.Rounded.ArrowBack, "Back") { vm.screen = Screen.HOME }
            Spacer(Modifier.width(12.dp))
            Text("Activity", color = air.text, fontWeight = FontWeight.Bold, fontSize = 22.sp, modifier = Modifier.weight(1f))
            if (list.any { it.status.isFinal }) IconCircleButton(Icons.Rounded.DeleteSweep, "Clear finished") { vm.engine.clearHistory() }
        }
        if (list.isEmpty()) {
            Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                Icon(Icons.Rounded.Inbox, null, tint = air.faint, modifier = Modifier.size(56.dp))
                Spacer(Modifier.height(10.dp))
                Text("Transfers will show up here", color = air.muted)
            }
            return
        }
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            items(list, key = { it.id }) { t -> TransferCard(t, vm, Modifier.animateItem()) }
            item { Spacer(Modifier.navigationBarsPadding()) }
        }
    }
}

private fun open(context: Context, file: TFile, vm: AppViewModel) {
    val uri = file.uri ?: return
    val intent = Intent(Intent.ACTION_VIEW).setDataAndType(Uri.parse(uri), file.mime)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (_: ActivityNotFoundException) {
        vm.toast("No app can open ${file.name}", good = false)
    }
}

private fun shareOn(context: Context, files: List<TFile>) {
    val uris = ArrayList(files.mapNotNull { it.uri?.let(Uri::parse) })
    if (uris.isEmpty()) return
    val intent = if (uris.size == 1) {
        Intent(Intent.ACTION_SEND).setType(files.first().mime).putExtra(Intent.EXTRA_STREAM, uris.first())
    } else {
        Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*").putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
    }
    context.startActivity(Intent.createChooser(intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION), null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

@Composable
private fun TransferCard(t: Transfer, vm: AppViewModel, modifier: Modifier) {
    val air = LocalAir.current
    val context = LocalContext.current
    GlassCard(modifier.fillMaxWidth().animateContentSize()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box {
                Avatar(t.peerId, deviceIcon(t.peerType, t.peerModel), 40.dp)
                Box(
                    Modifier.align(Alignment.BottomEnd).size(18.dp).clip(CircleShape).background(air.solid),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(if (t.direction == Direction.OUT) Icons.Rounded.ArrowUpward else Icons.Rounded.ArrowDownward, null, tint = Cyan, modifier = Modifier.size(12.dp))
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                val title = when {
                    t.message != null -> if (t.direction == Direction.OUT) "Message to ${t.peerAlias}" else "Message from ${t.peerAlias}"
                    t.files.size == 1 -> t.files.first().name
                    else -> "${t.files.size} files"
                }
                Text(title, color = air.text, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    when (t.status) {
                        Status.DONE -> Pill(if (t.direction == Direction.OUT) "Sent" else "Received", Mint)
                        Status.TRANSFERRING -> Pill(if (t.direction == Direction.OUT) "Sending" else "Receiving", Cyan)
                        Status.WAITING -> Pill("Waiting", Cyan)
                        else -> Pill(t.status.name.lowercase().replaceFirstChar { it.uppercase() }, Rose)
                    }
                    Spacer(Modifier.width(8.dp))
                    val detail = when (t.status) {
                        Status.TRANSFERRING -> "${formatBytes(t.done)} of ${formatBytes(t.total)} · ${formatBytes(t.speed)}/s"
                        Status.WAITING -> if (t.direction == Direction.OUT) "Waiting for ${t.peerAlias}…" else "Waiting for you"
                        Status.DONE -> (if (t.message == null) "${formatBytes(t.total)} · " else "") + timeAgo(t.finished ?: t.created)
                        else -> "${t.error ?: ""} · ${timeAgo(t.finished ?: t.created)}"
                    }
                    Text(detail, color = air.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
        if (t.message != null) {
            Spacer(Modifier.height(10.dp))
            Text(
                t.message,
                color = air.text,
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(topStart = 14.dp, topEnd = 14.dp, bottomEnd = 14.dp, bottomStart = 4.dp))
                    .background(air.surfaceStrong)
                    .padding(12.dp),
            )
        }
        val images = t.files.filter { it.mime.startsWith("image/") && it.uri != null && it.status == FileStatus.DONE }.take(4)
        if (images.isNotEmpty()) {
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                images.forEach { Thumbnail(it) { open(context, it, vm) } }
            }
        } else if (t.message == null && t.files.size > 1) {
            Spacer(Modifier.height(8.dp))
            t.files.take(4).forEach { f ->
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(vertical = 2.dp)) {
                    Icon(fileIcon(f.mime, f.name), null, tint = air.muted, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(f.name, color = air.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    Text(formatBytes(f.size), color = air.faint, fontSize = 12.sp)
                }
            }
            if (t.files.size > 4) Text("+ ${t.files.size - 4} more", color = air.faint, fontSize = 12.sp)
        }
        if (t.status == Status.TRANSFERRING) {
            Spacer(Modifier.height(10.dp))
            val progress by animateFloatAsState(t.progress, tween(300), label = "progress")
            Box(Modifier.fillMaxWidth().height(6.dp).clip(RoundedCornerShape(50)).background(air.surfaceStrong)) {
                Box(Modifier.fillMaxHeight().fillMaxWidth(progress).clip(RoundedCornerShape(50)).background(AccentGradient))
            }
        }
        val received = t.direction == Direction.IN && t.status == Status.DONE && t.files.any { it.uri != null }
        if (!t.status.isFinal || received || t.message != null) {
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (t.direction == Direction.IN && t.status == Status.WAITING) {
                    GradientButton("Review", null, height = 36.dp) { vm.showIncoming(t.id) }
                }
                if (!t.status.isFinal) GhostButton(if (t.status == Status.WAITING && t.direction == Direction.IN) "Decline" else "Cancel", null, height = 36.dp) { vm.engine.cancel(t.id) }
                if (received) {
                    GhostButton(if (t.files.size == 1) "Open" else "Downloads", null, height = 36.dp) {
                        if (t.files.size == 1) open(context, t.files.first(), vm)
                        else context.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    }
                    GhostButton("Share", null, height = 36.dp) { shareOn(context, t.files) }
                }
                if (t.message != null) GhostButton("Copy", null, height = 36.dp) {
                    vm.engine.copyToClipboard(t.message)
                    vm.toast("Copied")
                }
            }
        }
    }
}

@Composable
private fun Thumbnail(file: TFile, onClick: () -> Unit) {
    val context = LocalContext.current
    var bitmap by remember(file.uri) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(file.uri) {
        bitmap = withContext(Dispatchers.IO) {
            runCatching { context.contentResolver.loadThumbnail(Uri.parse(file.uri), Size(240, 240), null).asImageBitmap() }.getOrNull()
        }
    }
    val air = LocalAir.current
    Box(Modifier.size(68.dp).clip(RoundedCornerShape(12.dp)).background(air.surfaceStrong).pressable(onClick)) {
        bitmap?.let { Image(it, file.name, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
    }
}
