package io.github.mdhaarith.airbridge.core

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.os.StatFs
import android.provider.MediaStore
import android.provider.OpenableColumns
import java.io.IOException
import java.io.InputStream

/** Android file store: received files go to Download/AirBridge via MediaStore (no permission needed). */
class AndroidFileStore(private val context: Context) : FileStore {
    private val resolver get() = context.contentResolver

    override fun freeBytes(): Long = runCatching {
        StatFs(Environment.getExternalStorageDirectory().path).availableBytes
    }.getOrDefault(Long.MAX_VALUE)

    /** Creates a hidden (pending) entry; [publish] makes it visible once complete. */
    override fun create(fileName: String, mime: String): FileStore.Pending {
        val segments = safeSegments(fileName)
        val folder = (listOf(Environment.DIRECTORY_DOWNLOADS, "AirBridge") + segments.dropLast(1)).joinToString("/")
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, segments.last())
            put(MediaStore.MediaColumns.MIME_TYPE, mime.takeIf { '/' in it && '*' !in it } ?: "application/octet-stream")
            put(MediaStore.MediaColumns.RELATIVE_PATH, folder)
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: throw IOException("Cannot create ${segments.last()} in Downloads")
        // TLS hands over ~16 KB at a time; batch writes so shared storage sees few large ones.
        val output = resolver.openOutputStream(uri)?.buffered(1024 * 1024) ?: throw IOException("Cannot write ${segments.last()}")
        return FileStore.Pending(uri.toString(), output)
    }

    override fun publish(pending: FileStore.Pending) {
        pending.output.close()
        resolver.update(Uri.parse(pending.uri), ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
    }

    override fun discard(pending: FileStore.Pending) {
        runCatching { pending.output.close() }
        runCatching { resolver.delete(Uri.parse(pending.uri), null, null) }
    }

    /** Name, size and type of a shared/picked content URI. */
    override fun describe(uri: String): FileStore.Picked? {
        val parsed = Uri.parse(uri)
        var name = parsed.lastPathSegment?.substringAfterLast('/') ?: "file"
        var size = -1L
        runCatching {
            resolver.query(parsed, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }?.let { c.getString(it) }?.let { name = it }
                    c.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !c.isNull(it) }?.let { size = c.getLong(it) }
                }
            }
        }
        if (size < 0) {
            size = runCatching { resolver.openAssetFileDescriptor(parsed, "r")?.use { it.length } }.getOrNull() ?: -1
        }
        if (size < 0) return null
        val mime = resolver.getType(parsed) ?: android.webkit.MimeTypeMap.getSingleton()
            .getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"
        return FileStore.Picked(uri, name, size, mime)
    }

    override fun open(uri: String): InputStream = resolver.openInputStream(Uri.parse(uri)) ?: throw IOException("Cannot read $uri")
}
