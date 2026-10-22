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
        fun from(action: String?, streams: List<String>, text: CharSequence?, ownPackage: String = OWN_PACKAGE): ShareRequest? {
            if (action != SEND && action != SEND_MULTIPLE) return null
            val files = streams.filter { isShareable(it, ownPackage) }.distinct()
            if (files.isNotEmpty()) return ShareRequest(files, null)
            val clean = text?.toString()?.trim()?.takeIf { it.isNotEmpty() } ?: return null
            return ShareRequest(emptyList(), clean)
        }

        const val OWN_PACKAGE = "io.github.mdhaarith.airbridge"

        /**
         * Only content another app offers through a content:// address. A
         * file:// path or one of our own content addresses would make us send
         * our own private files (the identity key, settings) for whoever
         * crafted the intent.
         */
        fun isShareable(uri: String, ownPackage: String = OWN_PACKAGE): Boolean {
            if (!uri.startsWith("content://")) return false
            val authority = uri.removePrefix("content://").substringBefore('/').substringAfterLast('@')
            return authority.isNotEmpty() && !authority.startsWith(ownPackage)
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
