package io.github.mdhaarith.airbridge.core

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow

/** Owned by the QR screen; closing the screen or stopping the service removes the group. */
@SuppressLint("MissingPermission", "WakelockTimeout")
class WifiDirectCompanion(private val context: Context, private val engine: Engine) {
    data class State(val message: String = "Creating Wi-Fi Direct network…", val ssid: String = "", val password: String = "", val url: String = "")
    val state = MutableStateFlow(State())
    private val manager = context.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager
    private val handler = Handler(Looper.getMainLooper())
    private val channel = manager?.initialize(context, Looper.getMainLooper()) { stop("Wi-Fi Direct disconnected") }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var active = false
    private var creating = false
    private var disposed = false
    private var server: HttpServer? = null
    private var companion: BrowserCompanion? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var registered = false
    private val stopAction: () -> Unit = { handler.post { stop() }; Unit }
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context?, intent: Intent?) {
            if (intent?.action == WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION) inspect()
            if (intent?.action == WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION &&
                intent.getIntExtra(WifiP2pManager.EXTRA_WIFI_STATE, -1) == WifiP2pManager.WIFI_P2P_STATE_DISABLED) stop("Enable Wi-Fi to share")
        }
    }

    fun start() {
        if (active) return
        if (manager == null || channel == null) { state.value = State("Wi-Fi Direct is unavailable on this device"); return }
        active = true
        engine.stopCompanion = stopAction
        val filter = IntentFilter(WifiP2pManager.WIFI_P2P_CONNECTION_CHANGED_ACTION).apply { addAction(WifiP2pManager.WIFI_P2P_STATE_CHANGED_ACTION) }
        if (Build.VERSION.SDK_INT >= 33) context.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED) else context.registerReceiver(receiver, filter)
        registered = true
        scope.launch { delay(30 * 60_000L); stop("Session timed out") }
        scope.launch { delay(30_000); if (active && companion == null) stop("Could not create network. Enable Location on Android 12 or older, then retry.") }
        create(true)
    }

    private fun create(fiveGhz: Boolean) {
        if (!active || manager == null || channel == null) return
        creating = true
        val listener = object : WifiP2pManager.ActionListener {
            override fun onSuccess() { creating = false; if (active) inspect() else removeGroup() }
            override fun onFailure(reason: Int) {
                creating = false
                if (!active) { if (disposed) channel.close(); return }
                if (fiveGhz && Build.VERSION.SDK_INT >= 29) create(false)
                else stop("Could not create Wi-Fi Direct group (error $reason). Check Wi-Fi and Location settings.")
            }
        }
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                val credentials = credentials()
                val config = WifiP2pConfig.Builder().setNetworkName(credentials.first).setPassphrase(credentials.second)
                    .setGroupOperatingBand(operatingBand(fiveGhz)).build()
                manager.createGroup(channel, config, listener)
            } else manager.createGroup(channel, listener)
        } catch (e: Exception) { creating = false; stop(e.message ?: "Wi-Fi Direct permission denied") }
    }

    private fun inspect() {
        if (!active || manager == null || channel == null) return
        try {
            manager.requestGroupInfo(channel) { group ->
                if (!active) return@requestGroupInfo
                if (group == null) { if (companion != null) stop("Wi-Fi Direct group ended"); return@requestGroupInfo }
                if (!group.isGroupOwner) { stop("This phone must own the Wi-Fi Direct group"); return@requestGroupInfo }
                manager.requestConnectionInfo(channel) { info ->
                    if (!active || companion != null || !info.groupFormed || !info.isGroupOwner) return@requestConnectionInfo
                    val ip = info.groupOwnerAddress?.hostAddress ?: return@requestConnectionInfo
                    val ssid = group.networkName ?: return@requestConnectionInfo
                    val password = group.passphrase ?: return@requestConnectionInfo
                    val web = BrowserCompanion(engine) { context.assets.open(it).use { stream -> stream.readBytes() } }
                    companion = web
                    scope.launch {
                        try {
                            val http = HttpServer(null, web::handle)
                            server = http
                            val port = withContext(Dispatchers.IO) { http.start(53318, ip) }
                            if (!active) { http.stop(); return@launch }
                            wifiLock = (context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager)
                                .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "AirBridge:companion").apply { acquire() }
                            wakeLock = (context.getSystemService(Context.POWER_SERVICE) as PowerManager)
                                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "AirBridge:companion").apply { acquire(30 * 60_000L) }
                            state.value = State("Keep this screen open while sharing", ssid, password, "http://$ip:$port/#k=${web.secret}")
                        } catch (e: Exception) { stop(e.message ?: "Could not start companion") }
                    }
                }
            }
        } catch (e: Exception) { stop(e.message ?: "Wi-Fi Direct permission denied") }
    }

    fun offer(uris: List<String>) = companion?.offer(uris)

    fun stop(message: String = "Session stopped") {
        val wasActive = active
        active = false
        companion?.close(); companion = null
        server?.stop(); server = null
        scope.coroutineContext.cancelChildren()
        if (registered) { context.unregisterReceiver(receiver); registered = false }
        if (wasActive) removeGroup()
        wifiLock?.let { if (it.isHeld) it.release() }; wifiLock = null
        wakeLock?.let { if (it.isHeld) it.release() }; wakeLock = null
        if (engine.stopCompanion === stopAction) engine.stopCompanion = null
        state.value = State(message)
    }

    private fun removeGroup() {
        val p2p = manager ?: return
        val currentChannel = channel ?: return
        runCatching {
            p2p.removeGroup(currentChannel, object : WifiP2pManager.ActionListener {
                override fun onSuccess() { if (disposed && !creating) currentChannel.close() }
                override fun onFailure(reason: Int) {
                    if (!creating) state.value = state.value.copy(message = "Session stopped; Wi-Fi Direct cleanup failed ($reason). Turn Wi-Fi off if the network remains.")
                    if (disposed && !creating) currentChannel.close()
                }
            })
        }.onFailure { if (disposed && !creating) currentChannel.close() }
    }

    fun dispose() {
        disposed = true
        val wasActive = active
        stop()
        // Keep the channel alive until a pending create callback can remove its group.
        if (!wasActive && !creating) channel?.close()
        scope.cancel()
    }

    companion object {
        fun operatingBand(fiveGhz: Boolean) = if (fiveGhz) WifiP2pConfig.GROUP_OWNER_BAND_5GHZ else WifiP2pConfig.GROUP_OWNER_BAND_AUTO

        fun credentials(): Pair<String, String> {
            val token = BrowserCompanion.randomToken()
            return "DIRECT-${token.take(2)}-AirBridge" to token.substring(2, 26)
        }
        fun wifiQr(ssid: String, password: String): String {
            fun escape(s: String) = s.flatMap { if (it in "\\;,:\"") listOf('\\', it) else listOf(it) }.joinToString("")
            return "WIFI:T:WPA;S:${escape(ssid)};P:${escape(password)};;"
        }
    }
}
