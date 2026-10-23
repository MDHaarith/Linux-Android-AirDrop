package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.delay
import kotlinx.serialization.json.*
import java.io.IOException
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/** Browser protocol adapter; file acceptance and storage stay in the LocalSend engine. */
class BrowserCompanion(private val engine: Engine, private val asset: (String) -> ByteArray) {
    val secret = randomToken()
    private data class Upload(val owner: String, val session: PrepareUploadResponse)
    private val uploads = ConcurrentHashMap<String, Upload>()
    private val incomingPeers = ConcurrentHashMap.newKeySet<String>()
    private val offered = ConcurrentHashMap.newKeySet<String>()
    private val requests = RateLimit(20, 120, 60_000)
    @Volatile private var closed = false
    private fun json(value: JsonElement) = HttpResponse.json(200, value.toString())
    private fun hello() = buildJsonObject {
        put("alias", engine.me().alias); put("deviceModel", engine.me().deviceModel)
        put("poll", true); put("text", false)
    }

    fun offer(uris: List<String>): String? = engine.browserOffer(uris)?.also { offered.add(it) }

    fun close() {
        closed = true
        (offered + uploads.keys + engine.transfers.value.filter { it.peerId in incomingPeers && !it.status.isFinal }.map { it.id }).forEach { engine.finish(it, Status.CANCELLED, "Session ended") }
        uploads.clear()
        offered.clear()
    }

    suspend fun handle(req: HttpRequest): HttpResponse {
        if (closed) return HttpResponse.error(410, "Session ended")
        // The fragment cannot authenticate the first navigation. Only this inert bootstrap is public.
        if (req.method == "GET" && req.path == "/") return HttpResponse(200, BOOTSTRAP.toByteArray(), "text/html")
        if (!authorized(secret, req.query["k"])) return HttpResponse.error(403, "Invalid session key")
        if (req.method == "GET" && req.path == "/app") {
            val html = String(asset("companion.html"))
                .replace(Regex("(/assets/[^\" ]+)")) { "${it.value}?k=$secret" }
            return HttpResponse(200, html.toByteArray(), "text/html")
        }
        if (req.method == "GET" && req.path.startsWith("/assets/")) {
            val name = req.path.removePrefix("/assets/")
            if (name !in ASSETS) return HttpResponse.error(404, "Not found")
            val type = when (name.substringAfterLast('.')) { "js" -> "text/javascript"; "css" -> "text/css"; else -> "image/svg+xml" }
            return HttpResponse(200, asset(name), type)
        }
        if (req.path == "/hello" && req.method == "GET") return json(hello())
        val client = req.query["id"]?.takeIf { it.length in 1..64 && it.all { c -> c.isLetterOrDigit() } }
            ?: return HttpResponse.error(400, "Missing client id")
        if (req.path == "/poll" && req.method == "GET") {
            repeat(20) {
                if (closed) return HttpResponse.error(410, "Session ended")
                val cancelled = offered.mapNotNull(engine::transfer).firstOrNull { it.status in setOf(Status.CANCELLED, Status.FAILED, Status.DECLINED) }
                if (cancelled != null) {
                    offered.remove(cancelled.id)
                    return json(buildJsonArray { add(buildJsonObject { put("type", "cancel"); put("id", cancelled.id) }) })
                }
                val offers = offered.mapNotNull(engine::transfer).filter { it.status == Status.WAITING }
                if (offers.isNotEmpty()) return json(buildJsonArray { offers.take(1).forEach { t -> add(buildJsonObject {
                    put("type", "offer"); put("id", t.id); put("from", engine.me().alias)
                    putJsonArray("files") { t.files.forEach { f -> add(buildJsonObject {
                        put("id", f.id); put("name", f.name); put("size", f.size); put("type", f.mime)
                    }) } }
                }) } })
                delay(500)
            }
            return json(JsonArray(emptyList()))
        }
        if (req.path == "/send" && req.method == "POST") {
            uploads.entries.removeAll { engine.transfer(it.key)?.status?.isFinal != false }
            if (!requests.hit(req.remoteIp) || uploads.size >= 32) return HttpResponse.error(429, "Too many requests")
            val files = runCatching {
                val list = Protocol.json.parseToJsonElement(String(req.readBody())).jsonObject.getValue("files").jsonArray
                require(list.size in 1..Protocol.MAX_FILES)
                list.associate { item ->
                    val f = item.jsonObject
                    val id = UUID.randomUUID().toString()
                    val name = f.getValue("name").jsonPrimitive.content
                    require(name.isNotBlank() && name.length <= 1024)
                    id to FileDto(id, safeSegments(name).joinToString("/"), f.getValue("size").jsonPrimitive.long,
                        f["type"]?.jsonPrimitive?.content ?: "application/octet-stream")
                }
            }.getOrNull() ?: return HttpResponse.error(400, "Invalid files")
            val peerId = MessageDigest.getInstance("SHA-256").digest((secret + client).toByteArray()).joinToString("") { "%02x".format(it) }
            incomingPeers.add(peerId)
            if (closed) return HttpResponse.error(410, "Session ended")
            val response = engine.prepareUpload(req, PrepareUploadRequest(DeviceInfo("Phone (browser)", fingerprint = peerId, protocol = "http", deviceType = "web"), files))
            if (response.status != 200) return response
            val prepared = Protocol.json.decodeFromString(PrepareUploadResponse.serializer(), String(response.body))
            val transfer = engine.transfers.value.firstOrNull { t -> t.peerId == peerId && t.files.any { it.id in files } && t.status == Status.TRANSFERRING }
                ?: return HttpResponse.error(409, "Session ended")
            if (closed) { engine.cancel(transfer.id); return HttpResponse.error(410, "Session ended") }
            uploads[transfer.id] = Upload(client, prepared)
            return json(buildJsonObject {
                put("transfer", transfer.id)
                putJsonArray("files") { files.keys.forEach { add(buildJsonObject { put("id", it) }) } }
            })
        }
        if (req.path == "/upload" && req.method == "POST") {
            val entry = uploads[req.query["transfer"]] ?: return HttpResponse.error(403, "Unknown transfer")
            val file = req.query["file"] ?: return HttpResponse.error(400, "Missing file")
            if (entry.owner != client) return HttpResponse.error(403, "Wrong client")
            val token = entry.session.files[file] ?: return HttpResponse.error(403, "Unknown file")
            return engine.upload(HttpRequest(req.method, req.path, mapOf("sessionId" to entry.session.sessionId, "fileId" to file, "token" to token), req.headers, req.body, req.remoteIp, null) {})
        }
        val parts = req.path.split('/')
        if (parts.size == 4 && parts[1] == "offer" && req.method == "POST" && parts[2] in offered) {
            if (parts[3] !in setOf("accept", "decline")) return HttpResponse.error(400, "Invalid action")
            engine.browserAnswer(parts[2], parts[3] == "accept")
            return HttpResponse.json(200, "{}")
        }
        if (parts.size == 4 && parts[1] == "download" && req.method == "GET" && parts[2] in offered) {
            val t = engine.transfer(parts[2]) ?: return HttpResponse.error(404, "Not found")
            if (t.status != Status.TRANSFERRING && t.status != Status.DONE) return HttpResponse.error(409, "Not accepted")
            val f = t.files.firstOrNull { it.id == parts[3] } ?: return HttpResponse.error(404, "Not found")
            val range = byteRange(req.headers["range"], f.size)
            if (range == null) return HttpResponse(416, headers = mapOf("Content-Range" to "bytes */${f.size}"))
            val (start, count) = range
            val headers = mutableMapOf("Accept-Ranges" to "bytes", "Cache-Control" to "no-store",
                "Content-Disposition" to "attachment; filename=\"${safeSegments(f.name).last().map { if (it.code in 32..126 && it != '"' && it != '\\') it else '_' }.joinToString("")}\"")
            if (req.headers["range"] != null) headers["Content-Range"] = "bytes $start-${start + count - 1}/${f.size}"
            return HttpResponse(if (req.headers["range"] == null) 200 else 206, contentType = "application/octet-stream", headers = headers, length = count, stream = { out ->
                engine.browserOpen(f.uri!!).use { input ->
                    var skip = start
                    while (skip > 0) { val n = input.skip(skip); if (n > 0) skip -= n else if (input.read() >= 0) skip-- else throw IOException("Truncated file") }
                    val buffer = ByteArray(256 * 1024)
                    var remaining = count
                    while (remaining > 0) {
                        if (closed || engine.transfer(t.id)?.status?.isFinal == true && engine.transfer(t.id)?.status != Status.DONE) throw IOException("Cancelled")
                        val n = input.read(buffer, 0, minOf(buffer.size.toLong(), remaining).toInt())
                        if (n < 0) throw IOException("Truncated file")
                        out.write(buffer, 0, n); remaining -= n
                        if (start == 0L && t.status != Status.DONE) engine.progress(t.id, f.id, count - remaining)
                    }
                    out.flush()
                    // shortcut: resumed ranges do not complete transfer history; track coverage if resume completion reporting is needed.
                    if (start == 0L && count == f.size) engine.browserDownloaded(t.id, f.id)
                }
            })
        }
        return HttpResponse.error(404, "Not found")
    }

    companion object {
        private val ASSETS = setOf("shared.js", "shared.css", "companion.js", "companion.css", "icon.svg")
        private const val BOOTSTRAP = """<!doctype html><meta name="viewport" content="width=device-width"><title>AirBridge</title><script>const k=new URLSearchParams(location.hash.slice(1)).get('k');if(k){location.replace('/app?k='+encodeURIComponent(k)+'#k='+encodeURIComponent(k));}else{document.write('Scan the URL QR code in AirBridge.');}</script>"""
        fun randomToken(): String = ByteArray(24).also(SecureRandom()::nextBytes).joinToString("") { "%02x".format(it) }
        fun authorized(expected: String, supplied: String?) = supplied != null && MessageDigest.isEqual(expected.toByteArray(), supplied.toByteArray())
        /** Single HTTP byte range, represented as offset and length; null means 416. */
        fun byteRange(header: String?, size: Long): Pair<Long, Long>? {
            if (size < 0) return null
            if (header == null) return 0L to size
            val match = Regex("bytes=(\\d*)-(\\d*)").matchEntire(header) ?: return null
            val a = match.groupValues[1]; val b = match.groupValues[2]
            if (size == 0L) return null
            if (a.isEmpty()) {
                val suffix = b.toLongOrNull()?.takeIf { it > 0 } ?: return null
                val count = minOf(suffix, size)
                return size - count to count
            }
            val start = a.toLongOrNull()?.takeIf { it < size } ?: return null
            val end = if (b.isEmpty()) size - 1 else b.toLongOrNull()?.coerceAtMost(size - 1) ?: return null
            return if (end < start) null else start to end - start + 1
        }
    }
}
