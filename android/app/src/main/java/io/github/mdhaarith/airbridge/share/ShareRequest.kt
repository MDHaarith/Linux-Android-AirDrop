package io.github.mdhaarith.airbridge.share

/** What another app asked us to share, read from its Intent. */
data class ShareRequest(val uris: List<String>, val text: String?) {
    val isEmpty get() = uris.isEmpty() && text.isNullOrBlank()

    companion object {
        const val SEND = "android.intent.action.SEND"
        const val SEND_MULTIPLE = "android.intent.action.SEND_MULTIPLE"

        /**
         * Files win over text: apps often add a caption to shared photos.
         * Browsers share a page as EXTRA_TEXT = URL (the title goes in the
         * subject), so the text is sent as-is, like AirDrop sends a link.
         */
        fun from(action: String?, streams: List<String>, text: CharSequence?): ShareRequest? {
            if (action != SEND && action != SEND_MULTIPLE) return null
            val files = streams.filter { it.isNotBlank() }.distinct()
            if (files.isNotEmpty()) return ShareRequest(files, null)
            val clean = text?.toString()?.trim()?.takeIf { it.isNotEmpty() } ?: return null
            return ShareRequest(emptyList(), clean)
        }
    }
}

/** Direct Share targets are dynamic shortcuts named after the device. */
object ShortcutIds {
    private const val PREFIX = "device:"

    fun of(peerId: String) = PREFIX + peerId

    fun peerId(shortcutId: String?): String? =
        shortcutId?.takeIf { it.startsWith(PREFIX) }?.removePrefix(PREFIX)?.takeIf { it.isNotEmpty() }
}
