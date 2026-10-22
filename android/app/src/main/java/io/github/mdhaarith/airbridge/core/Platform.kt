package io.github.mdhaarith.airbridge.core

import java.io.File
import java.io.InputStream
import java.io.OutputStream

/**
 * Everything the engine needs from the operating system. The Android app
 * implements it with MediaStore, ClipboardManager and WifiManager; tests
 * implement it with plain files, so the networking core runs on a JVM too.
 */
interface Platform {
    /** Private app storage (identity, settings, history). */
    val filesDir: File
    /** Default name shown to other devices. */
    val defaultAlias: String
    val deviceModel: String
    val files: FileStore
    fun copyToClipboard(text: String)
    /** A small upright JPEG of the photo at [uri] (longest side about [Previews.SIZE]), or null. */
    fun thumbnail(uri: String, mime: String): ByteArray? = null
    /**
     * Devices at this phone's own address are refused (another app could pose
     * as one); tests running everything on one machine allow them.
     */
    val allowsLoopbackPeers: Boolean get() = false
    /** Lets multicast packets in (Android filters them otherwise); returns the release action. */
    fun acquireMulticastLock(): () -> Unit = {}
}

/** Where received files go, and how picked/shared files are read. URIs are plain strings. */
interface FileStore {
    data class Picked(val uri: String, val name: String, val size: Long, val mime: String)

    class Pending(val uri: String, val output: OutputStream, val handle: Any? = null)

    fun freeBytes(): Long
    fun create(fileName: String, mime: String): Pending
    fun publish(pending: Pending)
    fun discard(pending: Pending)
    fun describe(uri: String): Picked?
    fun open(uri: String): InputStream
}
