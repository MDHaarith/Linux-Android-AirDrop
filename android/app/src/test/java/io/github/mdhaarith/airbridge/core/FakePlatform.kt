package io.github.mdhaarith.airbridge.core

import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.InputStream
import java.util.Collections

/** A [Platform] backed by plain directories, so the engine runs on a JVM. */
class FakePlatform(root: File, alias: String) : Platform {
    override val filesDir = File(root, "files").apply { mkdirs() }
    override val defaultAlias = alias
    override val deviceModel = "JVM Test"
    override val allowsLoopbackPeers = true
    val downloads = File(root, "downloads").apply { mkdirs() }
    val clipboard: MutableList<String> = Collections.synchronizedList(mutableListOf())

    override val files = object : FileStore {
        override fun freeBytes() = Long.MAX_VALUE

        override fun create(fileName: String, mime: String): FileStore.Pending {
            val file = File(downloads, safeSegments(fileName).joinToString("/"))
            file.parentFile?.mkdirs()
            return FileStore.Pending(file.path, FileOutputStream(file))
        }

        override fun publish(pending: FileStore.Pending) = pending.output.close()

        override fun discard(pending: FileStore.Pending) {
            pending.output.close()
            File(pending.uri).delete()
        }

        override fun describe(uri: String): FileStore.Picked? {
            val file = File(uri)
            if (!file.isFile) return null
            return FileStore.Picked(uri, file.name, file.length(), "application/octet-stream")
        }

        override fun open(uri: String): InputStream = FileInputStream(uri)
    }

    override fun copyToClipboard(text: String) {
        clipboard += text
    }
}
