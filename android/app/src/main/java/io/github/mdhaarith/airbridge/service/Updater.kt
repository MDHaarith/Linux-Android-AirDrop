package io.github.mdhaarith.airbridge.service

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.widget.Toast
import androidx.core.content.IntentCompat
import io.github.mdhaarith.airbridge.BuildConfig
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.IOException

/**
 * Installs the newest AirBridge release from GitHub. Android refuses any APK that is not signed
 * with this app's key, and the session only accepts this package, so a swapped asset can't install.
 */
object Updater {
    private const val LATEST = "https://api.github.com/repos/MDHaarith/Linux-Android-AirDrop/releases/latest"
    private const val ASSET = "airbridge-android.apk"
    private const val MAX_SIZE = 200L * 1024 * 1024

    private val http = OkHttpClient()
    private val json = Json { ignoreUnknownKeys = true }

    @Serializable
    private class Asset(val name: String, @SerialName("browser_download_url") val url: String)

    @Serializable
    private class Release(
        @SerialName("tag_name") val tag: String,
        val draft: Boolean = false,
        val prerelease: Boolean = false,
        val assets: List<Asset> = emptyList(),
    )

    data class Available(val tag: String, val url: String)

    /** "v1.2.3" → [1, 2, 3]; anything else (pre-release suffixes included) → null. */
    fun version(tag: String): List<Int>? =
        tag.removePrefix("v").split('.').takeIf { it.size == 3 }?.map { it.toIntOrNull()?.takeIf { n -> n >= 0 } ?: return null }

    fun isNewer(tag: String, current: String): Boolean {
        val new = version(tag) ?: return false
        val old = version(current) ?: return true
        return new.zip(old).firstOrNull { (a, b) -> a != b }?.let { (a, b) -> a > b } ?: false
    }

    /** The latest stable release when it is newer than this build, else null. */
    suspend fun check(): Available? = withContext(Dispatchers.IO) {
        val body = http.newCall(Request.Builder().url(LATEST).header("Accept", "application/vnd.github+json").build()).execute().use {
            if (!it.isSuccessful) throw IOException("GitHub answered ${it.code}")
            it.body!!.string()
        }
        val release = json.decodeFromString<Release>(body)
        if (release.draft || release.prerelease || !isNewer(release.tag, BuildConfig.VERSION_NAME)) return@withContext null
        release.assets.firstOrNull { it.name == ASSET }?.let { Available(release.tag, it.url) }
    }

    /** Streams the APK into an install session; [UpdateReceiver] shows Android's confirmation. */
    suspend fun install(context: Context, update: Available, progress: (Float) -> Unit) = withContext(Dispatchers.IO) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setAppPackageName(context.packageName)
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            try {
                http.newCall(Request.Builder().url(update.url).build()).execute().use { response ->
                    if (!response.isSuccessful) throw IOException("download failed (${response.code})")
                    val body = response.body!!
                    val total = body.contentLength()
                    if (total > MAX_SIZE) throw IOException("the update is too large")
                    session.openWrite(ASSET, 0, total).use { out ->
                        body.byteStream().use { input ->
                            val buffer = ByteArray(64 * 1024)
                            var done = 0L
                            while (true) {
                                val n = input.read(buffer)
                                if (n < 0) break
                                done += n
                                if (done > MAX_SIZE) throw IOException("the update is too large")
                                out.write(buffer, 0, n)
                                if (total > 0) progress(done.toFloat() / total)
                            }
                        }
                        session.fsync(out)
                    }
                }
                // Mutable: the installer adds the status and its confirmation intent.
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
                session.commit(PendingIntent.getBroadcast(context, id, Intent(context, UpdateReceiver::class.java), flags).intentSender)
            } catch (e: Exception) {
                session.abandon()
                throw e
            }
        }
    }
}

/** The install session's result: ask the user to confirm, or say why it failed. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION ->
                IntentCompat.getParcelableExtra(intent, Intent.EXTRA_INTENT, Intent::class.java)
                    ?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            PackageInstaller.STATUS_SUCCESS -> Unit // Android restarts the app as the new version.
            PackageInstaller.STATUS_FAILURE_ABORTED -> Unit // The user tapped Cancel.
            else -> Toast.makeText(context, "Couldn't update: ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}", Toast.LENGTH_LONG).show()
        }
    }
}
