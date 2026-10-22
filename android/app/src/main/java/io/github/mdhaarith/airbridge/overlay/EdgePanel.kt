package io.github.mdhaarith.airbridge.overlay

import android.Manifest
import android.annotation.SuppressLint
import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PixelFormat
import android.graphics.RectF
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.provider.Settings
import android.util.Size
import android.view.DragEvent
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.WindowManager
import android.widget.FrameLayout
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.FileOpen
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Inbox
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.ComposeView
import androidx.compose.runtime.collectAsState
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.setViewTreeLifecycleOwner
import androidx.savedstate.SavedStateRegistryController
import androidx.savedstate.SavedStateRegistryOwner
import androidx.savedstate.setViewTreeSavedStateRegistryOwner
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.core.Log
import io.github.mdhaarith.airbridge.core.Peer
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.share.ShareSheetActivity
import io.github.mdhaarith.airbridge.share.ShareRequest
import io.github.mdhaarith.airbridge.ui.AirBridgeTheme
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
    private var handle: HandleView? = null
    private var panel: View? = null
    private var owner: OverlayOwner? = null

    // Panel state, read by the composable.
    private var clipboard by mutableStateOf<String?>(null)
    private var photo by mutableStateOf<Photo?>(null)
    private var dropped by mutableStateOf<Dropped?>(null)
    private var selected by mutableStateOf<String?>(null)
    private var targetId by mutableStateOf<String?>(null)
    private var transferId by mutableStateOf<String?>(null)
    private var error by mutableStateOf<String?>(null)

    private class Photo(val uri: Uri, val added: Long, val thumbnail: ImageBitmap?)

    private sealed interface Dropped {
        data class Text(val text: String) : Dropped
        data class Files(val uris: List<Uri>) : Dropped
    }

    val showing get() = handle != null

    fun show() {
        if (handle != null || !Settings.canDrawOverlays(context)) return
        val view = HandleView(context)
        val rightSide = engine.settings.value.edgeSide != "left"
        val metrics = context.resources.displayMetrics
        val params = WindowManager.LayoutParams(
            dp(26), dp(120),
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
            PixelFormat.TRANSLUCENT,
        ).apply {
            gravity = Gravity.TOP or if (rightSide) Gravity.END else Gravity.START
            y = (metrics.heightPixels * engine.settings.value.edgeY).toInt()
        }
        view.rightSide = rightSide
        runCatching { windows.addView(view, params) }
            .onSuccess { handle = view }
            .onFailure { Log.w(TAG, "could not add the edge handle", it) }
    }

    fun hide() {
        close()
        handle?.let { runCatching { windows.removeView(it) } }
        handle = null
    }

    fun destroy() {
        hide()
        scope.cancel()
    }

    // ---------------------------------------------------------------- panel

    fun open() {
        if (panel != null) return
        error = null
        targetId = null
        transferId = null
        loadPhoto()
        val owner = OverlayOwner().also { this.owner = it }
        val compose = ComposeView(context).apply {
            setViewTreeLifecycleOwner(owner)
            setViewTreeSavedStateRegistryOwner(owner)
            setContent {
                AirBridgeTheme {
                    val peers by engine.peers.collectAsState()
                    val transfers by engine.transfers.collectAsState()
                    val settings by engine.settings.state.collectAsState()
                    val ui = remember(peers, transfers, settings, clipboard, photo, dropped, selected, targetId, transferId, error) {
                        uiState(peers, settings.trusted, transfers.firstOrNull { it.id == transferId })
                    }
                    EdgePanelContent(ui, onSelect = ::select, onPick = ::send, onClose = ::close)
                }
            }
        }
        val root = object : FrameLayout(context) {
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
        root.addView(compose)
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
        dropped = null
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
                    !canReadPhotos() -> EdgeAction(PHOTO, "Latest photo", "Allow photo access in AirBridge settings", Icons.Rounded.Image, enabled = false)
                    photo == null -> EdgeAction(PHOTO, "Latest photo", "No photos yet", Icons.Rounded.Image, enabled = false)
                    else -> EdgeAction(PHOTO, "Latest photo", timeAgo(photo.added), Icons.Rounded.Image, thumbnail = photo.thumbnail)
                },
            )
            add(EdgeAction(FILES, "Choose files…", "Pick anything to send", Icons.Rounded.FileOpen))
        }

    /** The chosen action, or the first one that has something to send. */
    private fun selection(actions: List<EdgeAction>): String? =
        selected?.takeIf { key -> actions.any { it.key == key && it.enabled } }
            ?: actions.firstOrNull { it.enabled && it.key != FILES }?.key

    private fun uiState(peers: List<Peer>, trusted: Set<String>, transfer: io.github.mdhaarith.airbridge.core.Transfer?): EdgePanelUi {
        val actions = actions()
        val selected = selection(actions)
        val phase = when {
            error != null -> SharePhase.FAILED
            transfer == null -> SharePhase.PICK
            transfer.status == Status.WAITING -> SharePhase.WAITING
            transfer.status == Status.TRANSFERRING -> SharePhase.SENDING
            transfer.status == Status.DONE -> SharePhase.DONE
            else -> SharePhase.FAILED
        }
        return EdgePanelUi(
            rightSide = engine.settings.value.edgeSide != "left",
            actions = actions,
            selected = selected,
            peers = peers.sortedWith(compareByDescending<Peer> { it.id in trusted }.thenBy { it.alias.lowercase() }),
            trusted = trusted,
            targetId = targetId,
            phase = phase,
            progress = transfer?.progress ?: 0f,
            error = error ?: transfer?.takeIf { phase == SharePhase.FAILED }?.let { it.error ?: if (it.status == Status.DECLINED) "${it.peerAlias} declined" else "Couldn't send" },
        )
    }

    private fun select(key: String) {
        if (key == FILES) {
            close()
            context.startActivity(
                Intent(context, ShareSheetActivity::class.java)
                    .setAction(ShareSheetActivity.ACTION_PICK)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            return
        }
        selected = key
    }

    private fun send(peer: Peer) {
        val key = selection(actions()) ?: return
        error = null
        targetId = peer.id
        scope.launch {
            val id = withContext(Dispatchers.IO) {
                when (key) {
                    CLIPBOARD -> clipboard?.takeIf { it.isNotBlank() }?.let { engine.sendText(peer, it) }
                    PHOTO -> photo?.let { engine.sendFiles(peer, listOf(it.uri.toString())) }
                    DROPPED -> when (val d = dropped) {
                        is Dropped.Text -> engine.sendText(peer, d.text)
                        is Dropped.Files -> engine.sendFiles(peer, d.uris.map(Uri::toString))
                        null -> null
                    }
                    else -> null
                }
            }
            if (id == null) {
                error = "Nothing to send"
                return@launch
            }
            transferId = id
            val end = engine.transfers.first { list -> list.firstOrNull { it.id == id }?.status?.isFinal != false }
            if (end.firstOrNull { it.id == id }?.status == Status.DONE) {
                delay(1_400)
                if (transferId == id) close()
            }
        }
    }

    private fun readClipboard() {
        val manager = context.getSystemService(ClipboardManager::class.java)
        clipboard = runCatching { manager.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString() }.getOrNull()
    }

    private fun canReadPhotos(): Boolean {
        val permissions = when {
            Build.VERSION.SDK_INT >= 34 -> listOf(Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED)
            Build.VERSION.SDK_INT >= 33 -> listOf(Manifest.permission.READ_MEDIA_IMAGES)
            else -> listOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }
        return permissions.any { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }
    }

    private fun loadPhoto() {
        if (!canReadPhotos()) return
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

    /** Something was dropped on the handle (text, or files from another app). */
    private fun onDrop(clip: ClipData?) {
        clip ?: return
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
        val readable = uris.filter { uri -> runCatching { context.contentResolver.openInputStream(uri)?.close(); true }.getOrDefault(false) }
        dropped = when {
            readable.isNotEmpty() -> Dropped.Files(readable)
            texts.isNotEmpty() -> Dropped.Text(texts.joinToString("\n"))
            else -> null
        }
        if (dropped == null && uris.isNotEmpty()) error = "That app doesn't let others read its files. Use Share → AirBridge instead."
        selected = DROPPED.takeIf { dropped != null }
        open()
    }

    private fun saveHandlePosition(y: Int) {
        val height = context.resources.displayMetrics.heightPixels.coerceAtLeast(1)
        engine.settings.update { it.copy(edgeY = (y.toFloat() / height).coerceIn(0.05f, 0.9f)) }
    }

    private fun dp(value: Int) = (value * context.resources.displayMetrics.density).toInt()

    /** The handle: a glowing pill you can drag along the edge. */
    @SuppressLint("ViewConstructor", "ClickableViewAccessibility")
    private inner class HandleView(context: Context) : View(context) {
        var rightSide = true
        private var hovering = false
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val glow = Paint(Paint.ANTI_ALIAS_FLAG)
        private val slop = ViewConfiguration.get(context).scaledTouchSlop
        private var downX = 0f
        private var downY = 0f
        private var startY = 0
        private var dragging = false

        init {
            contentDescription = "AirBridge edge panel"
            setOnDragListener { _, event ->
                when (event.action) {
                    DragEvent.ACTION_DRAG_STARTED -> true
                    DragEvent.ACTION_DRAG_ENTERED -> { hovering = true; invalidate(); true }
                    DragEvent.ACTION_DRAG_EXITED, DragEvent.ACTION_DRAG_ENDED -> { hovering = false; invalidate(); true }
                    DragEvent.ACTION_DROP -> { hovering = false; invalidate(); onDrop(event.clipData); true }
                    else -> true
                }
            }
        }

        override fun onDraw(canvas: Canvas) {
            val w = width.toFloat()
            val h = height.toFloat()
            val pill = dp(if (hovering) 12 else 5).toFloat()
            val left = if (rightSide) w - pill - dp(2) else dp(2).toFloat()
            val rect = RectF(left, h * 0.08f, left + pill, h * 0.92f)
            // A quiet gray pill, like the home indicator; blue while something is dragged over it.
            paint.color = if (hovering) HANDLE_ACTIVE else HANDLE_IDLE
            if (hovering) {
                glow.color = HANDLE_ACTIVE
                glow.alpha = 60
                canvas.drawRoundRect(RectF(rect.left - dp(5), rect.top - dp(5), rect.right + dp(5), rect.bottom + dp(5)), pill, pill, glow)
            }
            canvas.drawRoundRect(rect, pill / 2, pill / 2, paint)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            val params = layoutParams as? WindowManager.LayoutParams ?: return false
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
                        runCatching { windows.updateViewLayout(this, params) }
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
        private const val HANDLE_ACTIVE = 0xFF007AFF.toInt()
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
