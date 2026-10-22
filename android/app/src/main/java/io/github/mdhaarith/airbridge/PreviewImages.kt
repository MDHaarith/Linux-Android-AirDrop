package io.github.mdhaarith.airbridge

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import io.github.mdhaarith.airbridge.core.Previews

/** Decodes the small photo previews other devices send, safely. */
object PreviewImages {
    /** Larger images than this (in pixels per side) are refused outright. */
    private const val MAX_SOURCE_SIDE = 8192

    /**
     * A preview's bytes as a bitmap no larger than [maxSide]: the size is
     * checked before decoding, since a tiny compressed file can claim a huge
     * picture (and a huge bitmap would crash the app).
     */
    fun decode(preview: String?, maxSide: Int = 720): Bitmap? {
        val bytes = Previews.decode(preview) ?: return null
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        val (w, h) = bounds.outWidth to bounds.outHeight
        if (w <= 0 || h <= 0 || w > MAX_SOURCE_SIDE || h > MAX_SOURCE_SIDE) return null
        var sample = 1
        while (maxOf(w, h) / (sample * 2) >= maxSide) sample *= 2
        return try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
        } catch (_: Throwable) {
            null
        }
    }
}
