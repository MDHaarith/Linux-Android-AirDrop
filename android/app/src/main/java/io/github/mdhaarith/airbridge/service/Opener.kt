package io.github.mdhaarith.airbridge.service

import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import io.github.mdhaarith.airbridge.core.Log
import io.github.mdhaarith.airbridge.core.Transfer

/**
 * Opens what your own devices send, like AirDrop between your devices: one
 * file in its app, several in Downloads, a link in the browser. Android may
 * not let a background app open things; the notification is still there.
 */
object Opener {
    private const val TAG = "Opener"

    fun openReceived(context: Context, t: Transfer) {
        val saved = t.files.filter { it.uri != null }
        val intent = when (saved.size) {
            0 -> return
            1 -> Intent(Intent.ACTION_VIEW).setDataAndType(Uri.parse(saved.first().uri), saved.first().mime)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            else -> Intent(DownloadManager.ACTION_VIEW_DOWNLOADS)
        }
        start(context, intent)
    }

    fun openLink(context: Context, text: String) {
        val link = text.trim()
        if (link.contains(Regex("\\s")) || !(link.startsWith("https://") || link.startsWith("http://"))) return
        start(context, Intent(Intent.ACTION_VIEW, Uri.parse(link)))
    }

    private fun start(context: Context, intent: Intent) {
        try {
            context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        } catch (e: ActivityNotFoundException) {
            Log.w(TAG, "nothing can open ${intent.data}", e)
        } catch (e: SecurityException) {
            Log.w(TAG, "not allowed to open ${intent.data}", e)
        }
    }
}
