package io.github.mdhaarith.airbridge

import android.app.Application
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Size
import io.github.mdhaarith.airbridge.core.AndroidFileStore
import io.github.mdhaarith.airbridge.core.Engine
import io.github.mdhaarith.airbridge.core.FileStore
import io.github.mdhaarith.airbridge.core.Platform
import io.github.mdhaarith.airbridge.core.Previews
import io.github.mdhaarith.airbridge.service.Notifications
import java.io.ByteArrayOutputStream
import java.io.File

class AirBridgeApp : Application() {
    val engine: Engine by lazy { Engine(AndroidPlatform(this)) }

    override fun onCreate() {
        super.onCreate()
        instance = this
        io.github.mdhaarith.airbridge.core.Log.sink = io.github.mdhaarith.airbridge.core.Log.Sink { priority, tag, message, error ->
            when (priority) {
                'D' -> android.util.Log.d(tag, message, error)
                'I' -> android.util.Log.i(tag, message, error)
                'W' -> android.util.Log.w(tag, message, error)
                else -> android.util.Log.e(tag, message, error)
            }
        }
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

    override fun thumbnail(uri: String, mime: String): ByteArray? {
        val source = Uri.parse(uri)
        val size = Previews.SIZE
        // The system's thumbnail when the provider has one, else a downscaled decode.
        val bitmap = runCatching { context.contentResolver.loadThumbnail(source, Size(size, size), null) }.getOrNull()
            ?: ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, source)) { decoder, info, _ ->
                val scale = size.toFloat() / maxOf(info.size.width, info.size.height)
                if (scale < 1f) decoder.setTargetSize((info.size.width * scale).toInt().coerceAtLeast(1), (info.size.height * scale).toInt().coerceAtLeast(1))
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            }
        val scale = size.toFloat() / maxOf(bitmap.width, bitmap.height)
        val fitted = if (scale < 1f) Bitmap.createScaledBitmap(bitmap, (bitmap.width * scale).toInt(), (bitmap.height * scale).toInt(), true) else bitmap
        return ByteArrayOutputStream().use { out ->
            fitted.compress(Bitmap.CompressFormat.JPEG, 72, out)
            out.toByteArray()
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
