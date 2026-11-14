@file:OptIn(
    androidx.compose.material3.ExperimentalMaterial3Api::class,
    androidx.compose.foundation.layout.ExperimentalLayoutApi::class,
)

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
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.rounded.ArrowDownward
import androidx.compose.material.icons.rounded.ArrowUpward
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.material3.*
import androidx.compose.material3.Icon
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
    val colors = MaterialTheme.colorScheme
    val transfers by vm.engine.transfers.collectAsStateWithLifecycle()
    val list = transfers.reversed()
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Activity") },
                navigationIcon = {
                    IconButton(onClick = { vm.screen = Screen.HOME }) {
                        Icon(Icons.AutoMirrored.Rounded.ArrowBack, "Back")
                    }
                },
                actions = {
                    if (list.any { it.status.isFinal })
                        TextButton(onClick = { vm.engine.clearHistory() }) { Text("Clear") }
                },
            )
        }
    ) { insets ->
        Box(Modifier.fillMaxSize().padding(insets)) {
            if (list.isEmpty()) {
                Column(
                    Modifier.fillMaxSize(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Icon(
                        Icons.Rounded.Inbox,
                        null,
                        tint = colors.onSurfaceVariant,
                        modifier = Modifier.size(52.dp),
                    )
                    Spacer(Modifier.height(10.dp))
                    Text(
                        "No activity",
                        color = colors.onSurface,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        "Things you send and receive show up here.",
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
                return@Scaffold
            }
            // Newest first, with a separate Material card for each transfer.
            LazyColumn(
                Modifier.fillMaxSize(),
                contentPadding = PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                itemsIndexed(list, key = { _, t -> t.id }) { _, t ->
                    Card(Modifier.animateItem().fillMaxWidth()) { TransferRow(t, vm) }
                }
            }
        }
    }
}

private fun open(context: Context, file: TFile, vm: AppViewModel) {
    val uri = file.uri ?: return
    val intent =
        Intent(Intent.ACTION_VIEW)
            .setDataAndType(Uri.parse(uri), file.mime)
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
    val intent =
        if (uris.size == 1) {
            Intent(Intent.ACTION_SEND)
                .setType(files.first().mime)
                .putExtra(Intent.EXTRA_STREAM, uris.first())
        } else {
            Intent(Intent.ACTION_SEND_MULTIPLE)
                .setType("*/*")
                .putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
        }
    context.startActivity(
        Intent.createChooser(intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION), null)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    )
}

@Composable
private fun TransferRow(t: Transfer, vm: AppViewModel) {
    val colors = MaterialTheme.colorScheme
    val context = LocalContext.current
    Column(
        Modifier.fillMaxWidth().animateContentSize().padding(horizontal = 16.dp, vertical = 16.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box {
                Avatar(deviceIcon(t.peerType, t.peerModel), 42.dp)
                Box(
                    Modifier.align(Alignment.BottomEnd)
                        .size(18.dp)
                        .clip(CircleShape)
                        .background(colors.surfaceContainerLow)
                        .padding(2.dp)
                        .clip(CircleShape)
                        .background(colors.primary),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        if (t.direction == Direction.OUT) Icons.Rounded.ArrowUpward
                        else Icons.Rounded.ArrowDownward,
                        null,
                        tint = colors.onPrimary,
                        modifier = Modifier.size(10.dp),
                    )
                }
            }
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                val title =
                    when {
                        t.message != null ->
                            if (t.direction == Direction.OUT) "Message to ${t.peerAlias}"
                            else "Message from ${t.peerAlias}"
                        t.files.size == 1 -> t.files.first().name
                        else -> "${t.files.size} files"
                    }
                Text(
                    title,
                    color = colors.onSurface,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    when (t.status) {
                        Status.DONE ->
                            Badge(
                                containerColor = colors.primaryContainer,
                                contentColor = colors.onPrimaryContainer,
                            ) {
                                Text(if (t.direction == Direction.OUT) "Sent" else "Received")
                            }
                        Status.TRANSFERRING ->
                            Badge(
                                containerColor = colors.primaryContainer,
                                contentColor = colors.onPrimaryContainer,
                            ) {
                                Text(if (t.direction == Direction.OUT) "Sending" else "Receiving")
                            }
                        Status.WAITING ->
                            Badge(
                                containerColor = colors.primaryContainer,
                                contentColor = colors.onPrimaryContainer,
                            ) {
                                Text("Waiting")
                            }
                        else ->
                            Badge(
                                containerColor = colors.errorContainer,
                                contentColor = colors.onErrorContainer,
                            ) {
                                Text(t.status.name.lowercase().replaceFirstChar { it.uppercase() })
                            }
                    }
                    Spacer(Modifier.width(8.dp))
                    val detail =
                        when (t.status) {
                            Status.TRANSFERRING ->
                                "${formatBytes(t.done)} of ${formatBytes(t.total)} · ${formatBytes(t.speed)}/s" +
                                    (t.elapsed?.let { " · ${formatDuration(it)}" } ?: "")
                            Status.WAITING ->
                                if (t.direction == Direction.OUT) "Waiting for ${t.peerAlias}…"
                                else "Waiting for you"
                            Status.DONE ->
                                (if (t.message == null) "${formatBytes(t.total)} · " else "") +
                                    (t.elapsed?.takeIf { t.message == null }?.let { "took ${formatDuration(it)} · " } ?: "") +
                                    timeAgo(t.finished ?: t.created)
                            else -> "${t.error ?: ""} · ${timeAgo(t.finished ?: t.created)}"
                        }
                    Text(
                        detail,
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        if (t.message != null) {
            Spacer(Modifier.height(10.dp))
            // A gray message bubble, like Messages.
            Text(
                t.message,
                color = colors.onSurface,
                style = MaterialTheme.typography.bodyLarge,
                modifier =
                    Modifier.clip(MaterialTheme.shapes.medium)
                        .background(colors.surfaceContainerHighest)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
            )
        }
        val images =
            t.files
                .filter {
                    it.mime.startsWith("image/") && it.uri != null && it.status == FileStatus.DONE
                }
                .take(4)
        if (images.isNotEmpty()) {
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                images.forEach { Thumbnail(it) { open(context, it, vm) } }
            }
        } else if (t.message == null && t.files.size > 1) {
            Spacer(Modifier.height(8.dp))
            t.files.take(4).forEach { f ->
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier.padding(vertical = 2.dp),
                ) {
                    Icon(
                        fileIcon(f.mime, f.name),
                        null,
                        tint = colors.onSurfaceVariant,
                        modifier = Modifier.size(16.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        f.name,
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        formatBytes(f.size),
                        color = colors.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
            if (t.files.size > 4)
                Text(
                    "+ ${t.files.size - 4} more",
                    color = colors.onSurfaceVariant,
                    style = MaterialTheme.typography.bodySmall,
                )
        }
        if (t.status == Status.TRANSFERRING) {
            Spacer(Modifier.height(10.dp))
            val progress by animateFloatAsState(t.progress, tween(300), label = "progress")
            LinearProgressIndicator(
                progress = { progress.coerceIn(0f, 1f) },
                modifier = Modifier.fillMaxWidth(),
            )
        }
        val received =
            t.direction == Direction.IN && t.status == Status.DONE && t.files.any { it.uri != null }
        if (!t.status.isFinal || received || t.message != null) {
            Spacer(Modifier.height(10.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (t.direction == Direction.IN && t.status == Status.WAITING) {
                    Button(
                        onClick = { vm.showIncoming(t.id) },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) {
                        Text("Review")
                    }
                }
                if (!t.status.isFinal)
                    FilledTonalButton(
                        onClick = { vm.engine.cancel(t.id) },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) {
                        Text(
                            if (t.status == Status.WAITING && t.direction == Direction.IN) "Decline"
                            else "Cancel"
                        )
                    }
                if (received) {
                    FilledTonalButton(
                        onClick = {
                            if (t.files.size == 1) open(context, t.files.first(), vm)
                            else
                                context.startActivity(
                                    Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)
                                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                                )
                        },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) {
                        Text(if (t.files.size == 1) "Open" else "Downloads")
                    }
                    FilledTonalButton(
                        onClick = { shareOn(context, t.files) },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) {
                        Text("Share")
                    }
                }
                if (t.message != null)
                    FilledTonalButton(
                        onClick = {
                            vm.engine.copyToClipboard(t.message)
                            vm.toast("Copied")
                        },
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) {
                        Text("Copy")
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
        bitmap =
            withContext(Dispatchers.IO) {
                runCatching {
                        context.contentResolver
                            .loadThumbnail(Uri.parse(file.uri), Size(240, 240), null)
                            .asImageBitmap()
                    }
                    .getOrNull()
            }
    }
    val colors = MaterialTheme.colorScheme
    Box(
        Modifier.size(68.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(colors.surfaceContainerHighest)
            .clickable(onClick = onClick)
    ) {
        bitmap?.let {
            Image(it, file.name, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        }
    }
}
