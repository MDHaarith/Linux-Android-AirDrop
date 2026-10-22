package io.github.mdhaarith.airbridge.share

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Shader
import androidx.compose.ui.graphics.toArgb
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import io.github.mdhaarith.airbridge.core.Log
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.ui.LightAir

/**
 * Nearby devices as Direct Share targets: they show up in the first row of
 * Android's share sheet, like AirDrop's row of nearby devices on an iPhone.
 * Picking one opens [ShareSheetActivity], which sends right away.
 */
object DirectShare {
    /** Must match the category in res/xml/shortcuts.xml. */
    const val CATEGORY = "io.github.mdhaarith.airbridge.category.DEVICE"
    private const val TAG = "DirectShare"

    private var planner: ShortcutPlanner? = null

    @Synchronized
    fun sync(context: Context, peers: List<Peer>, trusted: Set<String>) {
        val planner = planner ?: ShortcutPlanner(max = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context).coerceIn(1, 4))
            .also { planner = it }
        val plan = planner.update(peers, trusted, System.currentTimeMillis()) ?: return
        runCatching {
            if (plan.remove.isNotEmpty()) {
                val ids = plan.remove.map(ShortcutIds::of)
                ShortcutManagerCompat.removeDynamicShortcuts(context, ids)
                ShortcutManagerCompat.removeLongLivedShortcuts(context, ids)
            }
            plan.publish.forEachIndexed { rank, peer ->
                ShortcutManagerCompat.pushDynamicShortcut(context, shortcut(context, peer, rank))
            }
        }.onFailure { Log.w(TAG, "could not update share targets", it) }
    }

    private fun shortcut(context: Context, peer: Peer, rank: Int): ShortcutInfoCompat =
        ShortcutInfoCompat.Builder(context, ShortcutIds.of(peer.id))
            .setShortLabel(peer.alias.take(25))
            .setLongLabel("Send to ${peer.alias}")
            .setIcon(IconCompat.createWithAdaptiveBitmap(icon(context, peer)))
            // Also a launcher shortcut (long-press the app icon): pick files, then send.
            .setIntent(
                Intent(context, ShareSheetActivity::class.java)
                    .setAction(Intent.ACTION_SEND)
                    .putExtra(ShareSheetActivity.EXTRA_PEER, peer.id),
            )
            .setCategories(setOf(CATEGORY))
            .setLongLived(true)
            .setRank(rank)
            .build()

    /** An adaptive icon: a gray circle with the device's glyph, like its avatar in the app. */
    fun icon(context: Context, peer: Peer): Bitmap {
        val density = context.resources.displayMetrics.density
        val size = (108 * density).toInt().coerceAtLeast(108)
        val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        // The same gray as device avatars in the app.
        canvas.drawRect(
            0f, 0f, size.toFloat(), size.toFloat(),
            Paint().apply { shader = LinearGradient(0f, 0f, 0f, size.toFloat(), LightAir.avatarTop.toArgb(), LightAir.avatarBottom.toArgb(), Shader.TileMode.CLAMP) },
        )
        val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = android.graphics.Color.WHITE
            style = Paint.Style.STROKE
            strokeWidth = size * 0.035f
            strokeCap = Paint.Cap.ROUND
            strokeJoin = Paint.Join.ROUND
        }
        val c = size / 2f
        val u = size / 108f // one dp of the 108 dp canvas; the safe zone is the inner 72
        if (peer.deviceType == "mobile") {
            canvas.drawRoundRect(RectF(c - 9 * u, c - 15 * u, c + 9 * u, c + 15 * u), 4 * u, 4 * u, stroke)
            canvas.drawLine(c - 2.5f * u, c + 10 * u, c + 2.5f * u, c + 10 * u, stroke)
        } else {
            canvas.drawRoundRect(RectF(c - 14 * u, c - 11 * u, c + 14 * u, c + 7 * u), 3 * u, 3 * u, stroke)
            canvas.drawLine(c - 17 * u, c + 12 * u, c + 17 * u, c + 12 * u, stroke)
        }
        return bitmap
    }
}
