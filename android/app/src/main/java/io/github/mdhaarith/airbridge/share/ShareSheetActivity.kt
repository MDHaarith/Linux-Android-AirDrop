package io.github.mdhaarith.airbridge.share

import android.app.Application
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.util.Size
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.content.IntentCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.core.Log
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.service.AirBridgeService
import io.github.mdhaarith.airbridge.ui.AirBridgeTheme
import io.github.mdhaarith.airbridge.ui.SharePhase
import io.github.mdhaarith.airbridge.ui.SharePreview
import io.github.mdhaarith.airbridge.ui.ShareSheetContent
import io.github.mdhaarith.airbridge.ui.ShareSheetUi
import io.github.mdhaarith.airbridge.ui.formatBytes
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/**
 * "Share → AirBridge" (or a device straight from the share sheet's Direct
 * Share row): a sheet over the app you shared from that sends in one tap.
 */
class ShareSheetActivity : ComponentActivity() {
    private val vm: ShareSheetViewModel by viewModels()

    private val pickFiles = registerForActivityResult(ActivityResultContracts.GetMultipleContents()) { uris ->
        if (uris.isEmpty()) finish() else vm.begin(ShareRequest(uris.map(Uri::toString), null), vm.preselected)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handle(intent)
        setContent {
            AirBridgeTheme {
                val ui = vm.ui()
                LaunchedEffect(vm.finished) { if (vm.finished) finish() }
                BackHandler { dismiss(ui) }
                ShareSheetContent(
                    ui = ui,
                    onPick = vm::send,
                    onDismiss = { dismiss(ui) },
                    onRetry = vm::retry,
                    onScan = vm::scan,
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (!vm.isBusy()) handle(intent)
    }

    private fun handle(intent: Intent) {
        val peer = ShortcutIds.peerId(intent.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID))
            ?: intent.getStringExtra(EXTRA_PEER)
        val streams = when (intent.action) {
            Intent.ACTION_SEND_MULTIPLE -> IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
            else -> listOfNotNull(IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java))
        }
        val request = ShareRequest.from(intent.action, streams.map(Uri::toString), intent.getCharSequenceExtra(Intent.EXTRA_TEXT))
        vm.preselected = peer
        when {
            intent.action == ACTION_PICK -> pickFiles.launch("*/*")
            request != null -> vm.begin(request, peer)
            // A launcher shortcut ("Send to …"): choose what to send first.
            peer != null -> pickFiles.launch("*/*")
            else -> finish()
        }
    }

    /** While sending, step aside instead of stopping: the transfer goes on. */
    private fun dismiss(ui: ShareSheetUi) {
        if (ui.phase == SharePhase.WAITING || ui.phase == SharePhase.SENDING) moveTaskToBack(true) else finish()
    }

    companion object {
        const val EXTRA_PEER = "io.github.mdhaarith.airbridge.extra.PEER"
        /** Choose files first (from the edge panel), then a device. */
        const val ACTION_PICK = "io.github.mdhaarith.airbridge.action.PICK"
    }
}

class ShareSheetViewModel(app: Application) : AndroidViewModel(app) {
    private val engine = AirBridgeApp.engine
    var preselected: String? = null
    private var request: ShareRequest? = null
    private var preview by mutableStateOf(SharePreview("", ""))
    private var targetId by mutableStateOf<String?>(null)
    private var transferId by mutableStateOf<String?>(null)
    private var lookingFor by mutableStateOf<String?>(null)
    private var error by mutableStateOf<String?>(null)
    private var startedEngine = false
    var finished by mutableStateOf(false)
        private set

    @androidx.compose.runtime.Composable
    fun ui(): ShareSheetUi {
        val peers by engine.peers.collectAsState()
        val transfers by engine.transfers.collectAsState()
        val settings by engine.settings.state.collectAsState()
        val transfer = transferId?.let { id -> transfers.firstOrNull { it.id == id } }
        val phase = when {
            error != null -> SharePhase.FAILED
            transfer != null -> when (transfer.status) {
                Status.WAITING -> SharePhase.WAITING
                Status.TRANSFERRING -> SharePhase.SENDING
                Status.DONE -> SharePhase.DONE
                else -> SharePhase.FAILED
            }
            lookingFor != null -> SharePhase.LOOKING
            else -> SharePhase.PICK
        }
        val failure = error ?: transfer?.takeIf { phase == SharePhase.FAILED }?.let {
            when (it.status) {
                Status.DECLINED -> "${it.peerAlias} declined"
                Status.CANCELLED -> "Cancelled"
                else -> it.error ?: "Couldn't send to ${it.peerAlias}"
            }
        }
        return ShareSheetUi(
            preview = preview,
            peers = peers.sortedWith(compareByDescending<Peer> { it.id in settings.trusted }.thenBy { it.alias.lowercase() }),
            trusted = settings.trusted,
            targetId = targetId,
            lookingFor = lookingFor,
            phase = phase,
            progress = transfer?.progress ?: 0f,
            error = failure,
        )
    }

    fun isBusy(): Boolean =
        lookingFor != null || transferId?.let { engine.transfer(it) }?.status?.isFinal == false

    fun begin(request: ShareRequest, peerId: String?) {
        this.request = request
        targetId = null
        transferId = null
        error = null
        ensureEngine()
        viewModelScope.launch { preview = describe(request) }
        if (peerId != null) findAndSend(peerId)
    }

    private fun ensureEngine() {
        if (engine.running.value) return
        if (engine.settings.value.receiving) {
            AirBridgeService.start(getApplication())
        } else {
            // Only for this share; stopped again when the sheet goes away.
            startedEngine = true
            viewModelScope.launch(Dispatchers.IO) { runCatching { engine.start() }.onFailure { Log.w(TAG, "engine", it) } }
        }
    }

    private fun findAndSend(peerId: String) {
        lookingFor = ShortcutManagerCompat.getDynamicShortcuts(getApplication())
            .firstOrNull { ShortcutIds.peerId(it.id) == peerId }?.shortLabel?.toString() ?: "Device"
        viewModelScope.launch {
            val peer = withTimeoutOrNull(12_000) {
                engine.peers.first { list -> list.any { it.id == peerId } }.first { it.id == peerId }
            }
            val name = lookingFor
            lookingFor = null
            if (peer != null) send(peer) else error = "Couldn't find $name. Is it on this Wi‑Fi with AirBridge open?"
        }
    }

    fun send(peer: Peer) {
        val request = request ?: return
        targetId = peer.id
        error = null
        viewModelScope.launch {
            val id = withContext(Dispatchers.IO) {
                if (request.text != null) engine.sendText(peer, request.text) else engine.sendFiles(peer, request.uris)
            }
            if (id == null) error = "Couldn't read what you shared" else transferId = id
            if (id != null) awaitEnd(id)
        }
    }

    private suspend fun awaitEnd(id: String) {
        val done = engine.transfers.first { list -> list.firstOrNull { it.id == id }?.status?.isFinal != false }
            .firstOrNull { it.id == id }
        if (done?.status == Status.DONE) {
            delay(1_500) // long enough to see the check mark and the confetti
            finished = true
        }
    }

    fun retry() {
        val peer = engine.peers.value.firstOrNull { it.id == targetId }
        transferId = null
        error = null
        if (peer != null) send(peer) else targetId = null
    }

    fun scan() {
        engine.refresh()
        viewModelScope.launch { engine.scan() }
    }

    private suspend fun describe(request: ShareRequest): SharePreview = withContext(Dispatchers.IO) {
        val text = request.text
        if (text != null) {
            val link = Regex("^https?://\\S+$").matches(text)
            val host = if (link) runCatching { Uri.parse(text).host }.getOrNull() else null
            return@withContext SharePreview(
                title = host ?: if (link) "Link" else "Message",
                subtitle = text.take(140),
                isLink = link,
            )
        }
        val items = engine.describe(request.uris)
        val first = items.firstOrNull() ?: return@withContext SharePreview("Files", "${request.uris.size} items")
        val title = if (items.size == 1) first.name else when {
            items.all { it.mime.startsWith("image/") } -> "${items.size} photos"
            items.all { it.mime.startsWith("video/") } -> "${items.size} videos"
            else -> "${items.size} files"
        }
        val thumbnail = if (first.mime.startsWith("image/") || first.mime.startsWith("video/")) {
            runCatching {
                getApplication<Application>().contentResolver.loadThumbnail(Uri.parse(first.uri), Size(240, 240), null).asImageBitmap()
            }.getOrNull()
        } else {
            null
        }
        SharePreview(title, formatBytes(items.sumOf { it.size }), first.mime, thumbnail = thumbnail)
    }

    override fun onCleared() {
        if (startedEngine && !engine.settings.value.receiving) engine.stop()
        super.onCleared()
    }

    private companion object {
        const val TAG = "ShareSheet"
    }
}
