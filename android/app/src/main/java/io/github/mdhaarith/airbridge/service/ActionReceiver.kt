package io.github.mdhaarith.airbridge.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.widget.Toast
import androidx.core.app.NotificationManagerCompat
import io.github.mdhaarith.airbridge.AirBridgeApp

/** Handles Accept / Decline / Copy / Cancel buttons on notifications. */
class ActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(EXTRA_ID) ?: return
        val engine = AirBridgeApp.engine
        when (intent.action) {
            ACCEPT -> engine.decide(id, accept = true)
            DECLINE -> engine.decide(id, accept = false)
            CANCEL -> engine.cancel(id)
            COPY -> engine.transfer(id)?.message?.let {
                engine.copyToClipboard(it)
                Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show()
            }
        }
        if (intent.action != CANCEL) NotificationManagerCompat.from(context).cancel(Notifications.idFor(id))
    }

    companion object {
        const val EXTRA_ID = "id"
        const val ACCEPT = "io.github.mdhaarith.airbridge.ACCEPT"
        const val DECLINE = "io.github.mdhaarith.airbridge.DECLINE"
        const val COPY = "io.github.mdhaarith.airbridge.COPY"
        const val CANCEL = "io.github.mdhaarith.airbridge.CANCEL"
    }
}
