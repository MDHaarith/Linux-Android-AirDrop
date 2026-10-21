package io.github.mdhaarith.airbridge

import android.app.Application
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import io.github.mdhaarith.airbridge.core.AndroidFileStore
import io.github.mdhaarith.airbridge.core.Engine
import io.github.mdhaarith.airbridge.core.FileStore
import io.github.mdhaarith.airbridge.core.Platform
import io.github.mdhaarith.airbridge.service.Notifications
import java.io.File

class AirBridgeApp : Application() {
    val engine: Engine by lazy { Engine(AndroidPlatform(this)) }

    override fun onCreate() {
        super.onCreate()
        instance = this
        Notifications.createChannels(this)
    }

    companion object {
        lateinit var instance: AirBridgeApp
            private set
        val engine: Engine get() = instance.engine
    }
}

private class AndroidPlatform(private val context: Context) : Platform {
    override val filesDir: File = context.filesDir
    override val defaultAlias: String =
        Settings.Global.getString(context.contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL
    override val deviceModel: String = "${Build.MANUFACTURER.replaceFirstChar { it.uppercase() }} ${Build.MODEL}".trim()
    override val files: FileStore = AndroidFileStore(context)
    private val main = Handler(Looper.getMainLooper())

    override fun copyToClipboard(text: String) {
        main.post {
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("AirBridge", text))
        }
    }

    override fun acquireMulticastLock(): () -> Unit {
        val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        val lock = wifi.createMulticastLock("airbridge").apply {
            setReferenceCounted(false)
            acquire()
        }
        return { runCatching { lock.release() } }
    }
}
