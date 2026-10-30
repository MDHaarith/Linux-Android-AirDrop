package io.github.mdhaarith.airbridge.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.BuildConfig
import io.github.mdhaarith.airbridge.service.Updater
import kotlinx.coroutines.CancellationException
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Engine
import io.github.mdhaarith.airbridge.core.EngineEvent
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.FileStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

enum class Screen { HOME, ACTIVITY, SETTINGS, COMPANION }

/** Files or text waiting for the user to pick a device (AirDrop-style). */
data class PendingShare(val items: List<FileStore.Picked> = emptyList(), val text: String? = null) {
    val label: String
        get() = when {
            text != null -> "Message: ${text.take(40)}${if (text.length > 40) "…" else ""}"
            items.size == 1 -> items.first().name
            else -> "${items.size} items"
        }
    val totalBytes get() = items.sumOf { it.size }
}

data class Toast(val id: Long, val title: String, val text: String = "", val good: Boolean = true, val copyText: String? = null)

class AppViewModel(val engine: Engine = AirBridgeApp.engine) : ViewModel() {

    var screen by mutableStateOf(Screen.HOME)
    var pending by mutableStateOf<PendingShare?>(null)
        private set
    val incoming = mutableStateListOf<String>()
    val toasts = mutableStateListOf<Toast>()
    var scanning by mutableStateOf(false)
        private set
    var pinFor by mutableStateOf<Peer?>(null)

    init {
        viewModelScope.launch {
            engine.events.collect { event ->
                when (event) {
                    is EngineEvent.Incoming -> showIncoming(event.transferId)
                    is EngineEvent.Message -> toast(event.from, event.text, copyText = event.text.takeIf { !event.copied })
                    is EngineEvent.Finished -> onFinished(event)
                }
            }
        }
        // Requests that arrived while the UI was closed.
        engine.transfers.value.filter { it.direction == Direction.IN && it.status == Status.WAITING }.forEach { showIncoming(it.id) }
    }

    fun showIncoming(id: String) {
        if (id !in incoming) incoming += id
    }

    fun dismissIncoming(id: String) {
        incoming.remove(id)
    }

    private fun onFinished(event: EngineEvent.Finished) {
        val t = event.transfer
        if (t.direction == Direction.OUT) {
            when (t.status) {
                Status.DONE -> toast(if (t.message != null) "Message sent to ${t.peerAlias}" else "Sent to ${t.peerAlias}")
                Status.DECLINED -> toast("${t.peerAlias} declined", good = false)
                Status.FAILED -> {
                    toast("Couldn't send to ${t.peerAlias}", t.error ?: "", good = false)
                    if (t.error?.contains("PIN") == true) pinFor = engine.peers.value.firstOrNull { it.id == t.peerId }
                }
                else -> Unit
            }
        } else if (t.status == Status.DONE && t.message == null) {
            toast("Received from ${t.peerAlias}", "${if (t.files.size == 1) t.files.first().name else "${t.files.size} files"} · ${formatBytes(t.total)}")
        } else if (t.status == Status.FAILED) {
            toast("Transfer from ${t.peerAlias} failed", t.error ?: "", good = false)
        }
        incoming.remove(t.id)
    }

    /** What the Update row is doing ("Checking…", "Downloading 40%"), null when idle. */
    var updateStatus by mutableStateOf<String?>(null)

    /** Checks GitHub for a newer release and installs it. */
    fun updateApp(context: Context) {
        if (updateStatus != null) return
        updateStatus = "Checking…"
        viewModelScope.launch {
            try {
                val update = Updater.check()
                when {
                    update == null -> toast("AirBridge is up to date", "Version ${BuildConfig.VERSION_NAME}")
                    !context.packageManager.canRequestPackageInstalls() -> {
                        toast("Allow AirBridge to install apps", "Then tap Update again for ${update.tag}")
                        context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}")))
                    }
                    else -> Updater.install(context.applicationContext, update) { updateStatus = "Downloading ${(it * 100).toInt()}%" }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                toast("Couldn't update", e.message ?: "Check your connection", good = false)
            } finally {
                updateStatus = null
            }
        }
    }

    fun toast(title: String, text: String = "", good: Boolean = true, copyText: String? = null) {
        val toast = Toast(System.nanoTime(), title, text, good, copyText)
        toasts.add(0, toast)
        viewModelScope.launch {
            kotlinx.coroutines.delay(if (copyText != null) 12_000 else 4_500)
            toasts.remove(toast)
        }
    }

    fun stage(uris: List<Uri>) {
        viewModelScope.launch {
            val items = withContext(Dispatchers.IO) { engine.describe(uris.map { it.toString() }) }
            if (items.isNotEmpty()) pending = PendingShare(items = items)
            else toast("Couldn't read those files", good = false)
        }
    }

    fun stageText(text: String) {
        if (text.isNotBlank()) pending = PendingShare(text = text)
    }

    fun clearPending() {
        pending = null
    }

    /** Sends whatever is pending to [peer]. */
    fun sendPending(peer: Peer) {
        val share = pending ?: return
        pending = null
        if (share.text != null) engine.sendText(peer, share.text)
        else engine.sendFiles(peer, share.items.map { it.uri })
    }

    fun sendText(peer: Peer, text: String) {
        if (text.isNotBlank()) engine.sendText(peer, text)
    }

    fun sendFiles(peer: Peer, uris: List<Uri>) {
        if (uris.isEmpty()) return
        viewModelScope.launch(Dispatchers.IO) {
            if (engine.sendFiles(peer, uris.map { it.toString() }) == null) withContext(Dispatchers.Main) { toast("Couldn't read those files", good = false) }
        }
    }

    fun scan() {
        if (scanning) return
        scanning = true
        engine.refresh()
        viewModelScope.launch {
            val found = engine.scan()
            scanning = false
            toast(if (found > 0) "Found $found device${if (found == 1) "" else "s"}" else "No new devices found", if (found > 0) "" else "Is the other device on this Wi‑Fi with AirBridge open?", good = found > 0)
        }
    }
}
