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
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Status
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps AirBridge reachable (HTTPS server + discovery) while the app is in
 * the background, and turns engine events into notifications.
 */
class AirBridgeService : LifecycleService() {
    private val engine get() = AirBridgeApp.engine
    private val progressShown = mutableMapOf<String, Long>()

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
                        Notifications.finished(this@AirBridgeService, event.transfer)
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

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_STOP) {
            engine.settings.update { it.copy(receiving = false) }
            stopSelf()
            return START_NOT_STICKY
        }
        return START_STICKY
    }

    override fun onDestroy() {
        engine.stop()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "AirBridgeService"
        const val ACTION_STOP = "io.github.mdhaarith.airbridge.STOP"

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
