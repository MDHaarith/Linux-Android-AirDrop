package io.github.mdhaarith.airbridge.core

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.coroutines.resume

/** Joins another phone's Wi-Fi Direct network with the details from its QR, so nobody types or sees the password. */
@SuppressLint("MissingPermission")
class WifiDirectJoin(context: Context) {
    private val manager = context.getSystemService(Context.WIFI_P2P_SERVICE) as? WifiP2pManager
    private val channel = manager?.initialize(context, context.mainLooper, null)

    /** Null when joined, otherwise why it could not. */
    suspend fun join(ssid: String, password: String): String? {
        if (manager == null || channel == null) return "Wi-Fi Direct is unavailable on this device"
        val config = WifiP2pConfig.Builder().setNetworkName(ssid).setPassphrase(password).build()
        val refused = suspendCancellableCoroutine<Int?> { done ->
            manager.connect(channel, config, object : WifiP2pManager.ActionListener {
                override fun onSuccess() = done.resume(null)
                override fun onFailure(reason: Int) = done.resume(reason)
            })
        }
        if (refused != null) return "Could not join Wi-Fi Direct (error $refused). Check Wi-Fi and Location settings."
        val joined = withTimeoutOrNull(JOIN_TIMEOUT_MS) { while (!connected()) delay(500) }
        return if (joined != null) null else "The other phone's network did not answer. Stay close and try again."
    }

    suspend fun connected(): Boolean {
        if (manager == null || channel == null) return false
        return withTimeoutOrNull(3_000) {
            suspendCancellableCoroutine { done ->
                manager.requestConnectionInfo(channel) { done.resume(it?.groupFormed == true && !it.isGroupOwner) }
            }
        } ?: false
    }

    fun leave() {
        if (manager == null || channel == null) return
        val close = object : WifiP2pManager.ActionListener {
            override fun onSuccess() = channel.close()
            override fun onFailure(reason: Int) = channel.close()
        }
        runCatching { manager?.removeGroup(channel, close) }.onFailure { channel.close() }
    }

    private companion object {
        const val JOIN_TIMEOUT_MS = 30_000L
    }
}
