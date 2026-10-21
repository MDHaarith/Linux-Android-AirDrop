package io.github.mdhaarith.airbridge.service

import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.util.Log
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.overlay.EdgePanel
import io.github.mdhaarith.airbridge.share.DirectShare
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Keeps AirBridge reachable (HTTPS server + discovery) while the app is in
 * the background, and turns engine events into notifications.
 */
class AirBridgeService : LifecycleService() {
    private val engine get() = AirBridgeApp.engine
    private val progressShown = mutableMapOf<String, Long>()
    private var edgePanel: EdgePanel? = null

    override fun onCreate() {
        super.onCreate()
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE else 0
        ServiceCompat.startForeground(this, Notifications.ID_VISIBLE, Notifications.visible(this, engine.settings.value.alias), type)

        lifecycleScope.launch(Dispatchers.IO) {
            runCatching { engine.start() }.onFailure { Log.e(TAG, "could not start", it) }
        }
        lifecycleScope.launch {
            engine.events.collect { event ->
                when (event) {
                    is EngineEvent.Incoming -> engine.transfer(event.transferId)?.let { Notifications.incoming(this@AirBridgeService, it) }
                    is EngineEvent.Message -> Notifications.message(this@AirBridgeService, event.transferId, event.from, event.text, event.copied)
                    is EngineEvent.Finished -> {
                        progressShown.remove(event.transfer.id)
                        val t = event.transfer
                        // Received photos get a preview, like AirDrop's.
                        val picture = if (t.direction == Direction.IN && t.status == Status.DONE) {
                            withContext(Dispatchers.IO) { Notifications.thumbnail(this@AirBridgeService, t) }
                        } else {
                            null
                        }
                        Notifications.finished(this@AirBridgeService, t, picture)
                    }
                }
            }
        }
        lifecycleScope.launch {
            engine.transfers.collect { list ->
                val now = System.currentTimeMillis()
                for (t in list.filter { it.status == Status.TRANSFERRING }) {
                    if (now - (progressShown[t.id] ?: 0) > 700) {
                        progressShown[t.id] = now
                        Notifications.progress(this@AirBridgeService, t)
                    }
                }
            }
        }
        // Nearby devices as Direct Share targets in the system share sheet.
        lifecycleScope.launch(Dispatchers.Default) {
            engine.peers.combine(engine.settings.state.map { it.trusted }.distinctUntilChanged()) { peers, trusted -> peers to trusted }
                .collect { (peers, trusted) -> DirectShare.sync(this@AirBridgeService, peers, trusted) }
        }
        lifecycleScope.launch {
            engine.settings.state.map { it.edgePanel to it.edgeSide }.distinctUntilChanged().collect { updateEdgePanel(restart = true) }
        }
        lifecycleScope.launch {
            engine.settings.state.map { it.alias }.distinctUntilChanged().collect { alias ->
                androidx.core.app.NotificationManagerCompat.from(this@AirBridgeService).let { manager ->
                    if (manager.areNotificationsEnabled()) {
                        @android.annotation.SuppressLint("MissingPermission")
                        manager.notify(Notifications.ID_VISIBLE, Notifications.visible(this@AirBridgeService, alias))
                    }
                }
                engine.refresh()
            }
        }
    }

    /** Shows the edge handle when it is on and Android allows drawing over apps. */
    private fun updateEdgePanel(restart: Boolean = false) {
        val wanted = engine.settings.value.edgePanel && android.provider.Settings.canDrawOverlays(this)
        val panel = edgePanel ?: EdgePanel(this).also { edgePanel = it }
        if (restart || !wanted) panel.hide()
        if (wanted) panel.show()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_REFRESH_EDGE) updateEdgePanel()
        if (intent?.action == ACTION_STOP) {
            engine.settings.update { it.copy(receiving = false) }
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onDestroy() {
        edgePanel?.destroy()
        edgePanel = null
        engine.stop()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "AirBridgeService"
        const val ACTION_STOP = "io.github.mdhaarith.airbridge.STOP"
        const val ACTION_REFRESH_EDGE = "io.github.mdhaarith.airbridge.REFRESH_EDGE"

        /** Re-checks the edge panel, e.g. after the overlay permission was granted. */
        fun refreshEdgePanel(context: Context) {
            if (!AirBridgeApp.engine.settings.value.receiving) return
            ContextCompat.startForegroundService(context, Intent(context, AirBridgeService::class.java).setAction(ACTION_REFRESH_EDGE))
        }

        fun start(context: Context) {
            AirBridgeApp.engine.settings.update { it.copy(receiving = true) }
            ContextCompat.startForegroundService(context, Intent(context, AirBridgeService::class.java))
        }

        fun stop(context: Context) {
            AirBridgeApp.engine.settings.update { it.copy(receiving = false) }
            context.stopService(Intent(context, AirBridgeService::class.java))
        }
    }
}
