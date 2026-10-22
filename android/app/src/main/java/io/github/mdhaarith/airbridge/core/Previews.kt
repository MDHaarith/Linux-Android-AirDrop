package io.github.mdhaarith.airbridge.core

import java.util.Base64

/**
 * Small photo previews that travel with a request, so the other side can
 * show what is coming before accepting, like AirDrop does. LocalSend's
 * prepare-upload has an optional `preview` per file; for photos it carries a
 * `data:image/jpeg;base64,…` thumbnail (the Linux app does the same).
 */
object Previews {
    /** Previews we receive that are bigger than this are ignored. */
    const val MAX_DATA_URL = 300 * 1024
    /** How many files of a request get a preview. */
    const val MAX_PREVIEWS = 3
    /** Longest side of a preview we send, in pixels. */
    const val SIZE = 360

    private val accepted = listOf("data:image/jpeg;base64,", "data:image/png;base64,", "data:image/webp;base64,")

    fun isPreviewable(mime: String) = mime.startsWith("image/") && mime != "image/svg+xml"

    /** A received preview we are willing to show: a small raster image. */
    fun accept(preview: String?): String? =
        preview?.takeIf { it.length <= MAX_DATA_URL && accepted.any(it::startsWith) }

    /** The image bytes inside an accepted preview. */
    fun decode(preview: String?): ByteArray? {
        val data = accept(preview)?.substringAfter(',') ?: return null
        return runCatching { Base64.getDecoder().decode(data) }.getOrNull()
    }

    fun dataUrl(jpeg: ByteArray) = "data:image/jpeg;base64," + Base64.getEncoder().encodeToString(jpeg)
}

/** How AirDrop words a share: "a photo", "3 photos", "“Report.pdf”", "2 items". */
fun sharePhrase(files: List<TFile>): String {
    fun all(prefix: String) = files.isNotEmpty() && files.all { it.mime.startsWith(prefix) }
    return when {
        files.isEmpty() -> "something"
        files.size == 1 && all("image/") -> "a photo"
        files.size == 1 && all("video/") -> "a video"
        files.size == 1 -> "“${files.first().name.substringAfterLast('/')}”"
        all("image/") -> "${files.size} photos"
        all("video/") -> "${files.size} videos"
        else -> "${files.size} items"
    }
}
