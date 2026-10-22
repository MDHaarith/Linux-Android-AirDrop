package io.github.mdhaarith.airbridge.core

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.time.Instant
import java.time.format.DateTimeFormatter
import java.util.UUID

/** LocalSend protocol v2 (https://github.com/localsend/protocol). */
object Protocol {
    const val VERSION = "2.1"
    const val API = "/api/localsend/v2"
    const val DEFAULT_PORT = 53317
    const val MULTICAST_GROUP = "224.0.0.167"
    val DEVICE_TYPES = setOf("mobile", "desktop", "web", "headless", "server")
    /** Longest text message accepted (texts arrive inline in `preview`). */
    const val MAX_TEXT = 256 * 1024
    /** Most files one request may offer. */
    const val MAX_FILES = 10_000

    /**
     * A LocalSend device id: a certificate fingerprint, or a random token for
     * plain-HTTP peers. Ids of other kinds of peers cannot be claimed.
     */
    fun isPeerId(id: String) =
        id.length in 8..128 && !id.startsWith("web-") && id.all { it.isLetterOrDigit() && it.code < 128 || it == '-' || it == '_' }

    /** Bidi overrides and zero-width marks: invisible, but they change how names read. */
    fun isInvisible(c: Char) = c in '\u200B'..'\u200F' || c in '\u202A'..'\u202E' || c in '\u2060'..'\u2069' || c == '\uFEFF'

    fun stripInvisible(s: String) = s.filterNot(::isInvisible)

    val json = Json {
        ignoreUnknownKeys = true
        isLenient = true
        explicitNulls = true
        encodeDefaults = true
        coerceInputValues = true
    }
}

@Serializable
data class DeviceInfo(
    val alias: String,
    val version: String = "1.0",
    val deviceModel: String? = null,
    val deviceType: String? = "mobile",
    val fingerprint: String = "",
    val port: Int = Protocol.DEFAULT_PORT,
    val protocol: String = "https",
    val download: Boolean = false,
) {
    val isHttps get() = protocol != "http"

    /** Normalizes untrusted input. */
    fun sanitized(): DeviceInfo? {
        val name = Protocol.stripInvisible(alias.trim()).take(128)
        if (name.isEmpty()) return null
        return copy(
            alias = name,
            version = version.take(16),
            deviceModel = deviceModel?.let { Protocol.stripInvisible(it).take(128) },
            deviceType = deviceType?.takeIf { it in Protocol.DEVICE_TYPES } ?: "desktop",
            fingerprint = fingerprint.take(128),
            port = if (port in 1..65535) port else Protocol.DEFAULT_PORT,
            protocol = if (protocol == "http") "http" else "https",
        )
    }
}

@Serializable
data class MulticastDto(
    val alias: String,
    val version: String = "1.0",
    val deviceModel: String? = null,
    val deviceType: String? = null,
    val fingerprint: String = "",
    val port: Int = Protocol.DEFAULT_PORT,
    val protocol: String = "https",
    val download: Boolean = false,
    val announce: Boolean? = null,
    val announcement: Boolean? = null,
) {
    val isAnnouncement get() = announce ?: announcement ?: false
    fun info() = DeviceInfo(alias, version, deviceModel, deviceType, fingerprint, port, protocol, download)

    companion object {
        fun of(info: DeviceInfo, announce: Boolean) = MulticastDto(
            info.alias, info.version, info.deviceModel, info.deviceType, info.fingerprint,
            info.port, info.protocol, info.download, announce, announce,
        )
    }
}

/** Body of /register and /info responses (no port/protocol). */
@Serializable
data class InfoDto(
    val alias: String,
    val version: String,
    val deviceModel: String?,
    val deviceType: String?,
    val fingerprint: String,
    val download: Boolean = false,
)

@Serializable
data class FileMetadata(val modified: String? = null, val accessed: String? = null)

@Serializable
data class FileDto(
    val id: String,
    val fileName: String,
    val size: Long,
    val fileType: String = "application/octet-stream",
    val sha256: String? = null,
    val preview: String? = null,
    val metadata: FileMetadata? = null,
) {
    val isTextMessage get() = fileType.startsWith("text/") && preview != null

    fun sanitized(): FileDto? {
        if (id.isEmpty() || fileName.isEmpty() || size < 0) return null
        val type = when (fileType) {
            "image" -> "image/*"
            "video" -> "video/*"
            "pdf" -> "application/pdf"
            "text" -> "text/plain"
            "apk" -> "application/vnd.android.package-archive"
            "other", "" -> "application/octet-stream"
            else -> fileType.take(128)
        }
        // Text arrives inline; anything else may carry a small image preview.
        val text = type.startsWith("text") && preview != null
        if (text && preview!!.length > Protocol.MAX_TEXT) return null
        return copy(
            id = id.take(128),
            fileName = fileName.take(1024),
            fileType = type,
            preview = if (text) preview else Previews.accept(preview),
            sha256 = sha256?.takeIf { h -> h.length == 64 && h.all { it in '0'..'9' || it.lowercaseChar() in 'a'..'f' } },
        )
    }

    companion object {
        fun forText(text: String): FileDto {
            val id = UUID.randomUUID().toString()
            return FileDto(id, "$id.txt", text.toByteArray().size.toLong(), "text/plain", preview = text)
        }

        fun iso(millis: Long): String = DateTimeFormatter.ISO_INSTANT.format(Instant.ofEpochMilli(millis))
    }
}

@Serializable
data class PrepareUploadRequest(val info: DeviceInfo, val files: Map<String, FileDto>)

@Serializable
data class PrepareUploadResponse(val sessionId: String, val files: Map<String, String>)

@Serializable
data class ErrorDto(val message: String)

/** Turns a sender-supplied name (maybe with folders) into safe path segments. */
fun safeSegments(fileName: String): List<String> {
    val parts = fileName.replace('\\', '/').split('/')
        .map { part ->
            part.filter { !it.isISOControl() && !Protocol.isInvisible(it) }.trim().trimStart('.', ' ').let(::fitName).trimEnd()
        }
        .filter { it.isNotEmpty() && it != "." && it != ".." }
        .let { if (it.size > MAX_DEPTH) it.take(MAX_DEPTH - 1) + it.last() else it }
    return parts.ifEmpty { listOf("file") }
}

private const val MAX_DEPTH = 16

/** At most 200 bytes of UTF-8 (file systems allow 255), cut between characters. */
private fun fitName(name: String): String {
    var bytes = 0
    var end = 0
    while (end < name.length) {
        val codePoint = name.codePointAt(end)
        val size = String(Character.toChars(codePoint)).toByteArray().size
        if (bytes + size > 200) break
        bytes += size
        end += Character.charCount(codePoint)
    }
    return name.substring(0, end)
}
