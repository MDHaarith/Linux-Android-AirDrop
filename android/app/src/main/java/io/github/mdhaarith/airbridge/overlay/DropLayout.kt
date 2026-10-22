package io.github.mdhaarith.airbridge.overlay

import android.content.Context
import android.view.View
import android.widget.FrameLayout

/** A drag surface above the decorative children, using native View drag dispatch. */
internal open class DropLayout(context: Context) : FrameLayout(context) {
    val dropTarget = View(context)

    init {
        addView(dropTarget, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    }

    override fun onViewAdded(child: View) {
        super.onViewAdded(child)
        // ViewGroup dispatches to the frontmost interested child. The surface
        // must stay above Compose, including when the hidden card becomes visible.
        if (child !== dropTarget) dropTarget.bringToFront()
    }
}
