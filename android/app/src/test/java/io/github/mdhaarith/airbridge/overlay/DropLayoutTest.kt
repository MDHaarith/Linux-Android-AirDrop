package io.github.mdhaarith.airbridge.overlay

import android.content.ClipData
import android.net.Uri
import android.view.DragEvent
import android.view.View
import android.widget.FrameLayout
import app.cash.paparazzi.Paparazzi
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class DropLayoutTest {
    @get:Rule
    val paparazzi = Paparazzi()

    @Test
    fun deviceCardReceivesDropEvenWhenAChildHandlesDragEvents() {
        // A ViewGroup does not reach its own listener when an interested child
        // consumes DROP. Decorative children must not own the device card's drag.
        assertEquals(emptyList<String>(), dropOn(FrameLayout(paparazzi.context)))
        assertEquals(listOf("content://photos/42"), dropOn(DropLayout(paparazzi.context)))
    }

    private fun dropOn(root: FrameLayout): List<String> {
        val received = mutableListOf<String>()
        val target = if (root is DropLayout) root.dropTarget else root
        target.setOnReceiveContentListener(arrayOf("image/*")) { _, payload ->
            received += payload.clip.getItemAt(0).uri.toString()
            null
        }
        target.setOnDragListener { view, event ->
            if (event.action == DragEvent.ACTION_DROP) view.onDragEvent(event) else true
        }
        val child = View(paparazzi.context).apply {
            setOnDragListener { _, _ -> true }
        }
        root.addView(child, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        val clip = ClipData.newRawUri("photo", Uri.parse("content://photos/42"))
        root.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ ->
            for (action in listOf(DragEvent.ACTION_DRAG_STARTED, DragEvent.ACTION_DRAG_LOCATION, DragEvent.ACTION_DROP, DragEvent.ACTION_DRAG_ENDED)) {
                root.dispatchDragEvent(event(action, clip))
            }
        }
        paparazzi.snapshot(root)
        return received
    }

    // DragEvent.obtain is hidden from the SDK; use its fields to construct
    // platform events in the existing layoutlib runtime, without a new runner.
    private fun event(action: Int, clip: ClipData): DragEvent =
        DragEvent::class.java.getDeclaredConstructor().apply { isAccessible = true }.newInstance().also { event ->
            for ((name, value) in mapOf("mAction" to action, "mX" to 50f, "mY" to 50f, "mClipData" to clip, "mClipDescription" to clip.description)) {
                DragEvent::class.java.getDeclaredField(name).apply { isAccessible = true }.set(event, value)
            }
        }
}
