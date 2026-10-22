package io.github.mdhaarith.airbridge.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Size
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.github.mdhaarith.airbridge.MainActivity
import io.github.mdhaarith.airbridge.R
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Previews
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.core.sharePhrase
import io.github.mdhaarith.airbridge.ui.formatBytes

object Notifications {
    const val CHANNEL_VISIBLE = "visible"
    const val CHANNEL_REQUESTS = "requests"
    const val CHANNEL_TRANSFERS = "transfers"
    const val ID_VISIBLE = 1
    private const val ACCENT = 0xFF007AFF.toInt()

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannels(
            listOf(
                NotificationChannel(CHANNEL_VISIBLE, "Visible to nearby devices", NotificationManager.IMPORTANCE_MIN).apply {
                    description = "Shown while AirBridge can receive files"
                    setShowBadge(false)
                },
                NotificationChannel(CHANNEL_REQUESTS, "Incoming requests", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "Someone wants to share files with you"
                },
                NotificationChannel(CHANNEL_TRANSFERS, "Transfers", NotificationManager.IMPORTANCE_DEFAULT).apply {
                    description = "Progress and completed transfers"
                },
            ),
        )
    }

    private fun openApp(context: Context, transferId: String? = null): PendingIntent {
        val intent = Intent(context, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            .putExtra(MainActivity.EXTRA_TRANSFER, transferId)
        return PendingIntent.getActivity(context, transferId.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    private fun action(context: Context, action: String, transferId: String, requestCode: Int): PendingIntent {
        val intent = Intent(context, ActionReceiver::class.java).setAction(action).putExtra(ActionReceiver.EXTRA_ID, transferId)
        return PendingIntent.getBroadcast(context, requestCode, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    fun idFor(transferId: String) = 1000 + (transferId.hashCode() and 0xffff)

    fun visible(context: Context, alias: String): Notification =
        NotificationCompat.Builder(context, CHANNEL_VISIBLE)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ACCENT)
            .setContentTitle("Visible as “$alias”")
            .setContentText("Ready to receive from nearby devices")
            .setContentIntent(openApp(context))
            .setOngoing(true)
            .setSilent(true)
            .addAction(0, "Stop", PendingIntent.getService(
                context, 1, Intent(context, AirBridgeService::class.java).setAction(AirBridgeService.ACTION_STOP),
                PendingIntent.FLAG_IMMUTABLE,
            ))
            .build()

    private fun canPost(context: Context) = NotificationManagerCompat.from(context).areNotificationsEnabled()

    @android.annotation.SuppressLint("MissingPermission")
    fun incoming(context: Context, t: Transfer) {
        if (!canPost(context)) return
        // Worded like AirDrop, with the photo itself when the sender included a preview.
        val title = "“${t.peerAlias}” would like to share ${sharePhrase(t.files)}"
        val picture = Previews.decode(t.files.firstNotNullOfOrNull { it.preview })?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
        val style = if (picture != null) {
            NotificationCompat.BigPictureStyle().bigPicture(picture).bigLargeIcon(null as Bitmap?)
        } else {
            NotificationCompat.BigTextStyle().bigText(t.files.take(6).joinToString("\n") { "• ${it.name}" })
        }
        val notification = NotificationCompat.Builder(context, CHANNEL_REQUESTS)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ACCENT)
            .setContentTitle(title)
            .setContentText(formatBytes(t.total))
            .setLargeIcon(picture)
            .setStyle(style)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(openApp(context, t.id))
            .setFullScreenIntent(openApp(context, t.id), false)
            .setAutoCancel(true)
            .setTimeoutAfter(5 * 60_000)
            .addAction(0, "Decline", action(context, ActionReceiver.DECLINE, t.id, 2))
            .addAction(0, "Accept", action(context, ActionReceiver.ACCEPT, t.id, 3))
            .build()
        NotificationManagerCompat.from(context).notify(idFor(t.id), notification)
    }

    @android.annotation.SuppressLint("MissingPermission")
    fun message(context: Context, transferId: String, from: String, text: String, copied: Boolean) {
        if (!canPost(context)) return
        val builder = NotificationCompat.Builder(context, CHANNEL_TRANSFERS)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ACCENT)
            .setContentTitle(if (copied) "Copied text from $from" else "Message from $from")
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(openApp(context, transferId))
            .setAutoCancel(true)
        if (!copied) builder.addAction(0, "Copy", action(context, ActionReceiver.COPY, transferId, 4))
        NotificationManagerCompat.from(context).notify(idFor(transferId), builder.build())
    }

    @android.annotation.SuppressLint("MissingPermission")
    fun progress(context: Context, t: Transfer) {
        if (!canPost(context)) return
        val notification = NotificationCompat.Builder(context, CHANNEL_TRANSFERS)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ACCENT)
            .setContentTitle(if (t.direction == Direction.OUT) "Sending to ${t.peerAlias}" else "Receiving from ${t.peerAlias}")
            .setContentText("${formatBytes(t.done)} of ${formatBytes(t.total)} · ${formatBytes(t.speed)}/s")
            .setProgress(1000, (t.progress * 1000).toInt(), false)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setContentIntent(openApp(context, t.id))
            .addAction(0, "Cancel", action(context, ActionReceiver.CANCEL, t.id, 5))
            .build()
        NotificationManagerCompat.from(context).notify(idFor(t.id), notification)
    }

    /** A preview of the first received image or video, if there is one. */
    fun thumbnail(context: Context, t: Transfer): Bitmap? {
        val first = t.files.firstOrNull { (it.mime.startsWith("image/") || it.mime.startsWith("video/")) && it.uri != null } ?: return null
        return runCatching { context.contentResolver.loadThumbnail(Uri.parse(first.uri), Size(720, 720), null) }.getOrNull()
    }

    @android.annotation.SuppressLint("MissingPermission")
    fun finished(context: Context, t: Transfer, picture: Bitmap? = null) {
        val manager = NotificationManagerCompat.from(context)
        manager.cancel(idFor(t.id))
        if (!canPost(context) || t.message != null) return
        val title = when {
            t.status.name == "DONE" && t.direction == Direction.IN -> "Received ${if (t.files.size == 1) t.files.first().name else "${t.files.size} files"}"
            t.status.name == "DONE" -> "Sent to ${t.peerAlias}"
            t.status.name == "DECLINED" && t.direction == Direction.OUT -> "${t.peerAlias} declined"
            t.status.name == "FAILED" -> "Transfer failed"
            else -> return
        }
        val builder = NotificationCompat.Builder(context, CHANNEL_TRANSFERS)
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(ACCENT)
            .setContentTitle(title)
            .setContentText(if (t.direction == Direction.IN) "From ${t.peerAlias} · ${formatBytes(t.total)}" else t.error ?: formatBytes(t.total))
            .setAutoCancel(true)
        val first = t.files.firstOrNull()?.uri
        if (t.direction == Direction.IN && t.status.name == "DONE" && first != null) {
            val view = Intent(Intent.ACTION_VIEW).setDataAndType(Uri.parse(first), t.files.first().mime)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            builder.setContentIntent(PendingIntent.getActivity(context, idFor(t.id), view, PendingIntent.FLAG_IMMUTABLE))
            if (picture != null) {
                builder.setLargeIcon(picture)
                    .setStyle(NotificationCompat.BigPictureStyle().bigPicture(picture).bigLargeIcon(null as Bitmap?))
            }
            // Pass it on: Share opens the system share sheet with the received files.
            val uris = ArrayList(t.files.mapNotNull { it.uri }.map(Uri::parse))
            val mime = t.files.map { it.mime }.distinct().singleOrNull() ?: "*/*"
            val share = Intent(if (uris.size == 1) Intent.ACTION_SEND else Intent.ACTION_SEND_MULTIPLE).setType(mime)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                .apply { if (uris.size == 1) putExtra(Intent.EXTRA_STREAM, uris.first()) else putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris) }
            val chooser = Intent.createChooser(share, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            builder.addAction(0, "Share", PendingIntent.getActivity(context, idFor(t.id) + 1, chooser, PendingIntent.FLAG_IMMUTABLE))
            builder.addAction(0, "Show all", openApp(context, t.id))
        } else {
            builder.setContentIntent(openApp(context, t.id))
        }
        manager.notify(idFor(t.id), builder.build())
    }
}
