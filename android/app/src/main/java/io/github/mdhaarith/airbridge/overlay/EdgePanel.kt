package io.github.mdhaarith.airbridge.overlay

import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.RectF
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.provider.Settings
import android.util.Size
import android.view.ContentInfo
import android.view.DragEvent
import android.view.Gravity
import android.view.HapticFeedbackConstants
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.FileOpen
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.platform.ComposeView
import androidx.compose.runtime.collectAsState
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.MainActivity
import io.github.mdhaarith.airbridge.canReadPhotos
import io.github.mdhaarith.airbridge.core.Log
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.Transfer
import io.github.mdhaarith.airbridge.share.ShareSheetActivity
import io.github.mdhaarith.airbridge.share.ShareRequest
import io.github.mdhaarith.airbridge.ui.AirBridgeTheme
import io.github.mdhaarith.airbridge.ui.DropTab
import io.github.mdhaarith.airbridge.ui.DropTabLayout
import io.github.mdhaarith.airbridge.ui.DropTabUi
import io.github.mdhaarith.airbridge.ui.DropTarget
import io.github.mdhaarith.airbridge.ui.EdgeAction
import io.github.mdhaarith.airbridge.ui.EdgePanelContent
import io.github.mdhaarith.airbridge.ui.EdgePanelUi
import io.github.mdhaarith.airbridge.ui.SharePhase
import io.github.mdhaarith.airbridge.ui.timeAgo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.abs

/**
 * A slim handle on the screen edge, over every app. Tap or swipe it to open
 * a panel that sends the clipboard, your latest photo or something you
 * dropped on the handle, to a nearby device.
 */
class EdgePanel(private val context: Context) {
    private val engine = AirBridgeApp.engine
    private val windows = context.getSystemService(WindowManager::class.java)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var handle: HandleRoot? = null
    private var panel: View? = null
    private var owner: OverlayOwner? = null

    // Panel state, read by the composable.
    private var clipboard by mutableStateOf<String?>(null)
    private var photo by mutableStateOf<Photo?>(null)
    private var dropped by mutableStateOf<Dropped?>(null)
    private var selected by mutableStateOf<String?>(null)
    private var targetId by mutableStateOf<String?>(null)
    private var transferIds by mutableStateOf<List<String>>(emptyList())
    private var preparing by mutableStateOf(false)
    private var sendGeneration = 0L
    private var error by mutableStateOf<String?>(null)
    private var panelHover by mutableStateOf<String?>(null)

    // The drop tab the handle turns into while something is dragged.
    private var tabTargets by mutableStateOf<List<DropTarget>>(emptyList())
    private var tabHovered by mutableStateOf<String?>(null)
    private var tabOver by mutableStateOf(false)
    private val panelTargets = mutableMapOf<String, Rect>()

    // What keeps a drop's files readable (Android grants it with the drop and
    // takes it back once this is gone), held until the next drop.
    private var dropGrant: Any? = null

    private class Photo(val uri: Uri, val added: Long, val thumbnail: ImageBitmap?)

    private sealed interface Dropped {
        data class Text(val text: String) : Dropped
        data class Files(val uris: List<Uri>) : Dropped
    }

    val showing get() = handle != null

    fun show() {
        if (handle != null || !Settings.canDrawOverlays(context)) return
        val rightSide = engine.settings.value.edgeSide != "left"
        val view = runCatching { HandleRoot(context, rightSide) }.getOrElse {
            Log.w(TAG, "could not build the edge handle", it)
            return
        }
        val metrics = context.resources.displayMetrics
        val params = WindowManager.LayoutParams(
            dp(HANDLE_WIDTH), dp(HANDLE_HEIGHT),
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or if (rightSide) Gravity.END else Gravity.START
            y = (metrics.heightPixels * engine.settings.value.edgeY).toInt()
        }
        runCatching { windows.addView(view, params) }
            .onSuccess { handle = view }
            .onFailure { Log.w(TAG, "could not add the edge handle", it); view.destroy() }
    }

    fun hide() {
        close()
        handle?.let { runCatching { windows.removeView(it) }; it.destroy() }
        handle = null
    }

    fun destroy() {
        hide()
        scope.cancel()
    }

    // ---------------------------------------------------------------- panel

    fun open() {
        if (panel != null) return
        if (!busy()) {
            error = null
            targetId = null
            transferIds = emptyList()
        }
        loadPhoto()
        val owner = OverlayOwner().also { this.owner = it }
        val compose = ComposeView(context).apply {
            setContent {
                AirBridgeTheme {
                    val peers by engine.peers.collectAsState()
                    val transfers by engine.transfers.collectAsState()
                    val settings by engine.settings.state.collectAsState()
                    val ui = uiState(peers, settings.trusted, transfers)
                    EdgePanelContent(ui, onSelect = ::select, onPick = ::send, onClose = ::close,
                        onDefault = ::setDefault, onSendAll = ::sendAll,
                        onDropBounds = { id, bounds -> panelTargets[id] = bounds })
                }
            }
        }
        val root = object : DropLayout(context) {
            override fun dispatchKeyEvent(event: KeyEvent): Boolean {
                if (event.keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) {
                    close()
                    return true
                }
                return super.dispatchKeyEvent(event)
            }

            override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
                super.onWindowFocusChanged(hasWindowFocus)
                // Android only lets the app with input focus read the clipboard.
                if (hasWindowFocus) readClipboard()
            }
        }
        // Compose resolves its window recomposer from the overlay root.
        root.setViewTreeLifecycleOwner(owner)
        root.setViewTreeSavedStateRegistryOwner(owner)
        root.addView(compose)
        var receivingTarget: String? = null
        if (Build.VERSION.SDK_INT >= 31) {
            root.dropTarget.setOnReceiveContentListener(DROP_TYPES) { _, payload ->
                onDrop(payload.clip, grant = payload, target = receivingTarget)
                null
            }
        }
        root.dropTarget.setOnDragListener { view, event ->
            val target = panelTargets.entries.firstOrNull { (id, bounds) ->
                (id == DropTargets.ALL || engine.peers.value.any { it.id == id }) && bounds.contains(Offset(event.x, event.y))
            }?.key
            when (event.action) {
                DragEvent.ACTION_DRAG_STARTED -> !busy()
                DragEvent.ACTION_DRAG_ENTERED, DragEvent.ACTION_DRAG_LOCATION -> { panelHover = target; true }
                DragEvent.ACTION_DRAG_EXITED, DragEvent.ACTION_DRAG_ENDED -> { panelHover = null; true }
                DragEvent.ACTION_DROP -> {
                    receivingTarget = target
                    panelHover = null
                    if (Build.VERSION.SDK_INT < 31 || !view.onDragEvent(event)) onDrop(event.clipData, target = target)
                    true
                }
                else -> true
            }
        }
        val params = WindowManager.LayoutParams(
            WindowManager.LayoutParams.MATCH_PARENT, WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,
            PixelFormat.TRANSLUCENT,
        )
        runCatching { windows.addView(root, params) }
            .onSuccess { panel = root }
            .onFailure { Log.w(TAG, "could not open the edge panel", it); owner.destroy() }
        engine.refresh()
    }

    fun close() {
        panel?.let { runCatching { windows.removeView(it) } }
        panel = null
        owner?.destroy()
        owner = null
        panelHover = null
        panelTargets.clear()
        if (!busy()) {
            dropped = null
            dropGrant = null
        }
    }

    private fun actions(): List<EdgeAction> = buildList {
            dropped?.let {
                add(
                    when (it) {
                        is Dropped.Text -> EdgeAction(DROPPED, "What you dropped", it.text.take(60), Icons.Rounded.Inbox)
                        is Dropped.Files -> EdgeAction(DROPPED, "What you dropped", "${it.uris.size} item${if (it.uris.size == 1) "" else "s"}", Icons.Rounded.Inbox)
                    },
                )
            }
            add(EdgeAction(CLIPBOARD, "Clipboard", clipboard?.take(80)?.replace('\n', ' ') ?: "Nothing copied", Icons.Rounded.ContentPaste, enabled = !clipboard.isNullOrBlank()))
            val photo = photo
            add(
                when {
                    !canReadPhotos(context) -> EdgeAction(PHOTO, "Latest photo", "Tap to allow photo access", Icons.Rounded.Image)
                    photo == null -> EdgeAction(PHOTO, "Latest photo", "No photos yet", Icons.Rounded.Image, enabled = false)
                    else -> EdgeAction(PHOTO, "Latest photo", timeAgo(photo.added), Icons.Rounded.Image, thumbnail = photo.thumbnail)
                },
            )
            add(EdgeAction(FILES, "Choose files…", "Pick anything to send", Icons.Rounded.FileOpen))
        }

    /** The chosen action, or the first one that has something to send. */
    private fun selection(actions: List<EdgeAction>): String? =
        selected?.takeIf { key -> actions.any { it.key == key && it.enabled } && ready(key) }
            ?: actions.firstOrNull { it.enabled && it.key != FILES && ready(it.key) }?.key

    /** Whether choosing [key] sends something now (rather than asking for access first). */
    private fun ready(key: String) = key != PHOTO || (canReadPhotos(context) && photo != null)

    private fun busy() = preparing || engine.transfers.value.any { it.id in transferIds && !it.status.isFinal }

    private fun setDefault(id: String?) {
        engine.settings.update { it.copy(edgeRecipient = id) }
    }

    private fun uiState(peers: List<Peer>, trusted: Set<String>, transfers: List<Transfer>): EdgePanelUi {
        val actions = actions()
        val selected = selection(actions)
        val batch = transfers.filter { it.id in transferIds }
        val phase = when {
            preparing -> SharePhase.WAITING
            batch.any { it.status == Status.TRANSFERRING } -> SharePhase.SENDING
            batch.any { it.status == Status.WAITING } -> SharePhase.WAITING
            error != null -> SharePhase.FAILED
            batch.isEmpty() -> SharePhase.PICK
            batch.all { it.status == Status.DONE } -> SharePhase.DONE
            else -> SharePhase.FAILED
        }
        return EdgePanelUi(
            rightSide = engine.settings.value.edgeSide != "left",
            actions = actions,
            selected = selected,
            peers = peers.sortedWith(compareByDescending<Peer> { it.id in trusted }.thenBy { it.alias.lowercase() }),
            trusted = trusted,
            targetId = targetId,
            defaultTarget = engine.settings.value.edgeRecipient,
            hoverTarget = panelHover,
            phase = phase,
            progress = if (batch.isEmpty()) 0f else batch.map { it.progress }.average().toFloat(),
            error = error ?: batch.firstOrNull { it.status.isFinal && it.status != Status.DONE }?.let { it.error ?: if (it.status == Status.DECLINED) "${it.peerAlias} declined" else "Couldn't send to ${it.peerAlias}" },
        )
    }

    private fun select(key: String) {
        when {
            key == FILES -> pickFiles(null)
            key == PHOTO && !canReadPhotos(context) -> {
                close()
                context.startActivity(
                    Intent(context, MainActivity::class.java)
                        .setAction(MainActivity.ACTION_ASK_PHOTOS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
            else -> selected = key
        }
    }

    /** Choose files in the system picker, then send them (to [peer] if given). */
    private fun pickFiles(peer: Peer?) {
        close()
        context.startActivity(
            Intent(context, ShareSheetActivity::class.java)
                .setAction(ShareSheetActivity.ACTION_PICK)
                .apply { if (peer != null) putExtra(ShareSheetActivity.EXTRA_PEER, peer.id) }
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }

    private fun send(peer: Peer) {
        setDefault(peer.id)
        sendTo(listOf(peer), peer.id)
    }

    private fun sendAll() {
        setDefault(DropTargets.ALL)
        sendTo(engine.peers.value.toList(), DropTargets.ALL)
    }

    private fun sendTo(peers: List<Peer>, target: String) {
        if (busy()) return
        if (peers.isEmpty()) { error = "No nearby devices to send to"; return }
        // Nothing copied, no photo: a device still does something, it asks what to send.
        val key = selection(actions()) ?: run {
            if (peers.size == 1) pickFiles(peers.first()) else error = "Choose something to send first, then Send to all"
            return
        }
        val text = clipboard
        val image = photo
        val payload = dropped
        val grant = dropGrant
        error = null
        targetId = target
        preparing = true
        transferIds = emptyList()
        val generation = ++sendGeneration
        scope.launch {
            try {
                val ids = mutableListOf<String>()
                for (peer in peers) {
                    try {
                        val id = withContext(Dispatchers.IO) {
                            when (key) {
                                CLIPBOARD -> text?.takeIf { it.isNotBlank() }?.let { engine.sendText(peer, it) }
                                PHOTO -> image?.let { engine.sendFiles(peer, listOf(it.uri.toString())) }
                                DROPPED -> when (payload) {
                                    is Dropped.Text -> engine.sendText(peer, payload.text)
                                    is Dropped.Files -> engine.sendFiles(peer, payload.uris.map(Uri::toString))
                                    null -> null
                                }
                                else -> null
                            }
                        }
                        if (id == null) error = "Couldn't prepare files for ${peer.alias}"
                        else { ids += id; transferIds = ids.toList() }
                    } catch (e: Exception) {
                        if (e is kotlinx.coroutines.CancellationException) throw e
                        error = "Couldn't send to ${peer.alias}: ${e.message ?: "try again"}"
                    }
                }
                preparing = false
                if (ids.isEmpty()) { error = "Nothing to send"; return@launch }
                val end = engine.transfers.first { list -> ids.all { id -> list.firstOrNull { it.id == id }?.status?.isFinal == true } }
                if (error == null && ids.all { id -> end.first { it.id == id }.status == Status.DONE }) {
                    delay(1_400)
                    if (transferIds == ids) close()
                }
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) throw e
                if (sendGeneration == generation) error = e.message ?: "Couldn't send"
            } finally {
                if (sendGeneration == generation) preparing = false
                if (dropGrant === grant && !busy()) dropGrant = null
            }
        }
    }

    private fun readClipboard() {
        val manager = context.getSystemService(ClipboardManager::class.java)
        clipboard = runCatching { manager.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString() }.getOrNull()
    }

    private fun loadPhoto() {
        if (!canReadPhotos(context)) return
        scope.launch {
            photo = withContext(Dispatchers.IO) {
                runCatching {
                    context.contentResolver.query(
                        MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                        arrayOf(MediaStore.Images.Media._ID, MediaStore.Images.Media.DATE_ADDED),
                        null, null,
                        "${MediaStore.Images.Media.DATE_ADDED} DESC",
                    )?.use { cursor ->
                        if (!cursor.moveToFirst()) return@use null
                        val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, cursor.getLong(0))
                        val thumbnail = runCatching { context.contentResolver.loadThumbnail(uri, Size(160, 160), null).asImageBitmap() }.getOrNull()
                        Photo(uri, cursor.getLong(1) * 1000, thumbnail)
                    }
                }.getOrNull()
            }
        }
    }

    /**
     * Something was dropped on the handle (text, or files from another app).
     * [grant] keeps the other app's files readable while we hold on to it.
     */
    private fun onDrop(clip: ClipData?, grant: Any? = null, target: String? = null) {
        clip ?: return
        if (busy()) return
        open()
        dropGrant = grant
        val texts = mutableListOf<String>()
        val uris = mutableListOf<Uri>()
        for (i in 0 until clip.itemCount) {
            val item = clip.getItemAt(i)
            // Only content another app hands over, never our own private files.
            item.uri?.let { uri -> if (ShareRequest.isShareable(uri.toString())) uris += uri }
                ?: item.text?.toString()?.takeIf { it.isNotBlank() }?.let(texts::add)
        }
        // Files dragged from another app are only readable with a permission
        // that overlay windows cannot ask for; check before offering them.
        val readable = uris.filter { uri -> runCatching { context.contentResolver.openInputStream(uri)?.use { true } == true }.getOrDefault(false) }
        dropped = when {
            readable.isNotEmpty() -> Dropped.Files(readable)
            texts.isNotEmpty() -> Dropped.Text(texts.joinToString("\n"))
            else -> null
        }
        error = if (dropped == null && uris.isNotEmpty()) "That app doesn't let others read its files. Use Share → AirBridge instead." else null
        selected = DROPPED.takeIf { dropped != null }
        if (dropped != null) {
            val destination = target ?: engine.settings.value.edgeRecipient
            val peers = DropTargets.resolve(engine.peers.value, destination)
            if (peers.isNotEmpty()) sendTo(peers, destination ?: peers.single().id)
            else error = if (destination != null) "Default device is offline. Choose a nearby device." else "Drop on a device bubble or choose a default."
        }
    }

    private fun saveHandlePosition(y: Int) {
        val height = context.resources.displayMetrics.heightPixels.coerceAtLeast(1)
        engine.settings.update { it.copy(edgeY = (y.toFloat() / height).coerceIn(0.05f, 0.9f)) }
    }

    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).toInt()

    /**
     * The handle's window: a quiet pill you can drag along the edge
     * ([HandleView]). While something is dragged anywhere on screen it grows
     * into a card listing nearby devices ([DropTab]); dropping on a row sends
     * there.
     */
    @SuppressLint("ViewConstructor")
    private inner class HandleRoot(context: Context, private val rightSide: Boolean) : DropLayout(context) {
        private val pill = HandleView(context, rightSide, this)
        private val owner = OverlayOwner()
        private val tab = ComposeView(context).apply {
            visibility = GONE
            setContent {
                AirBridgeTheme {
                    val settings by engine.settings.state.collectAsState()
                    DropTab(DropTabUi(rightSide, tabTargets, tabHovered, tabOver, settings.edgeRecipient))
                }
            }
        }
        private var expanded = false
        private var restY = 0
        private var receivingTarget: String? = null

        init {
            setViewTreeLifecycleOwner(owner)
            setViewTreeSavedStateRegistryOwner(owner)
            contentDescription = "AirBridge edge panel"
            addView(pill, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            addView(tab, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
            if (Build.VERSION.SDK_INT >= 31) {
                // Android reads a drop for us here and lends us the right to
                // open the other app's files, which an overlay can't ask for itself.
                // (Android refuses a bare "*/*" here, so name the families.)
                dropTarget.setOnReceiveContentListener(DROP_TYPES) { _, payload: ContentInfo ->
                    onDrop(payload.clip, grant = payload, target = receivingTarget)
                    null
                }
            }
            dropTarget.setOnDragListener { view, event ->
                when (event.action) {
                    DragEvent.ACTION_DRAG_STARTED -> if (busy()) false else { startDrag(); true }
                    DragEvent.ACTION_DRAG_ENTERED, DragEvent.ACTION_DRAG_LOCATION -> {
                        val target = targetAt(event.y)
                        // A light tick each time the finger reaches a new device, like moving between AirDrop bubbles.
                        if (target != null && target != tabHovered) performHapticFeedback(HapticFeedbackConstants.CLOCK_TICK)
                        tabOver = true
                        tabHovered = target
                        true
                    }
                    DragEvent.ACTION_DRAG_EXITED -> { tabOver = false; tabHovered = null; true }
                    DragEvent.ACTION_DRAG_ENDED -> { tabOver = false; tabHovered = null; expand(false); true }
                    DragEvent.ACTION_DROP -> {
                        receivingTarget = targetAt(event.y)
                        tabOver = false
                        tabHovered = null
                        // Android 12+ passes the drop, readable, to the content listener above.
                        if (Build.VERSION.SDK_INT < 31 || !view.onDragEvent(event)) onDrop(event.clipData, target = receivingTarget)
                        true
                    }
                    else -> true
                }
            }
        }

        /** Lists the devices to drop on: your default first, then by name; everyone when there are several. */
        private fun startDrag() {
            val metrics = context.resources.displayMetrics
            val rows = ((metrics.heightPixels / metrics.density - DropTabLayout.HEADER - DropTabLayout.FOOTER - 40) / DropTabLayout.ROW).toInt().coerceAtLeast(1)
            val settings = engine.settings.value
            val devices = engine.peers.value.distinctBy { it.id }
                .sortedWith(compareByDescending<Peer> { it.id == settings.edgeRecipient }.thenByDescending { it.id in settings.trusted }.thenBy { it.alias.lowercase() })
            val everyone = if (devices.size > 1) listOf(DropTarget(DropTargets.ALL, "Send to all", "${devices.size} devices nearby", everyone = true)) else emptyList()
            tabTargets = (everyone + devices.map { peer ->
                DropTarget(peer.id, peer.alias, if (peer.id in settings.trusted) "My device" else peer.deviceModel, peer.deviceType, peer.deviceModel)
            }).take(rows)
            engine.refresh()
            expand(true)
        }

        private fun targetAt(y: Float): String? =
            DropTabLayout.rowAt(y / context.resources.displayMetrics.density, tabTargets.size)?.let { tabTargets[it].id }

        /** A sliver at rest; a card that is easy to hit while something is being dragged. */
        private fun expand(on: Boolean) {
            if (expanded == on) return
            val params = layoutParams as? WindowManager.LayoutParams ?: return
            if (on) restY = params.y
            val count = tabTargets.size
            params.width = dp(if (on) DropTabLayout.width(count) else HANDLE_WIDTH)
            params.height = dp(if (on) DropTabLayout.height(count) else HANDLE_HEIGHT)
            val screen = context.resources.displayMetrics.heightPixels
            params.y = if (on) (restY - (params.height - dp(HANDLE_HEIGHT)) / 2).coerceIn(0, (screen - params.height).coerceAtLeast(0)) else restY
            expanded = on
            pill.visibility = if (on) GONE else VISIBLE
            tab.visibility = if (on) VISIBLE else GONE
            runCatching { windows.updateViewLayout(this, params) }
        }

        fun destroy() = owner.destroy()
    }

    /** The resting handle: a quiet gray pill, like the home indicator. Tap or swipe it in to open the panel; drag it along the edge to move it. */
    @SuppressLint("ViewConstructor", "ClickableViewAccessibility")
    private inner class HandleView(context: Context, private val rightSide: Boolean, private val window: View) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = HANDLE_IDLE }
        private val slop = ViewConfiguration.get(context).scaledTouchSlop
        private var downX = 0f
        private var downY = 0f
        private var startY = 0
        private var dragging = false

        override fun onDraw(canvas: Canvas) {
            val pillWidth = dp(5).toFloat()
            val left = if (rightSide) width - pillWidth - dp(2) else dp(2).toFloat()
            canvas.drawRoundRect(RectF(left, height * 0.08f, left + pillWidth, height * 0.92f), pillWidth / 2, pillWidth / 2, paint)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            val params = window.layoutParams as? WindowManager.LayoutParams ?: return false
            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    downX = event.rawX
                    downY = event.rawY
                    startY = params.y
                    dragging = false
                }
                MotionEvent.ACTION_MOVE -> {
                    val dx = event.rawX - downX
                    val dy = event.rawY - downY
                    val inward = if (rightSide) -dx else dx
                    if (!dragging && inward > slop * 2 && abs(dy) < inward) {
                        dragging = true
                        open()
                    } else if (dragging || abs(dy) > slop) {
                        dragging = true
                        params.y = (startY + dy).toInt().coerceAtLeast(0)
                        runCatching { windows.updateViewLayout(window, params) }
                    }
                }
                MotionEvent.ACTION_UP -> {
                    if (!dragging) {
                        performClick()
                    } else if (params.y != startY) {
                        saveHandlePosition(params.y)
                    }
                }
            }
            return true
        }

        override fun performClick(): Boolean {
            super.performClick()
            open()
            return true
        }
    }

    /** ComposeView needs a lifecycle and saved state, even outside an Activity. */
    private class OverlayOwner : SavedStateRegistryOwner {
        private val registry = LifecycleRegistry(this)
        private val saved = SavedStateRegistryController.create(this)
        override val lifecycle: Lifecycle get() = registry
        override val savedStateRegistry get() = saved.savedStateRegistry

        init {
            saved.performRestore(null)
            registry.currentState = Lifecycle.State.RESUMED
        }

        fun destroy() {
            registry.currentState = Lifecycle.State.DESTROYED
        }
    }

    companion object {
        private const val TAG = "EdgePanel"
        private const val HANDLE_IDLE = 0x998E8E93.toInt()
        private const val HANDLE_WIDTH = 26
        private const val HANDLE_HEIGHT = 120
        private val DROP_TYPES = arrayOf("image/*", "video/*", "audio/*", "text/*", "application/*", "font/*", "model/*")
        private const val CLIPBOARD = "clipboard"
        private const val PHOTO = "photo"
        private const val DROPPED = "dropped"
        private const val FILES = "files"

        /** Opens the system screen where the user allows drawing over other apps. */
        fun permissionIntent(context: Context): Intent =
            Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${context.packageName}"))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
