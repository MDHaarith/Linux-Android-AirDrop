package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

data class Peer(
    val id: String,
    val alias: String,
    val deviceModel: String?,
    val deviceType: String,
    val ip: String,
    val port: Int,
    val https: Boolean,
    val lastSeen: Long,
    val failures: Int = 0,
    /** Its id was proven by its TLS certificate, not just claimed in a packet. */
    val verified: Boolean = false,
)

@Serializable
enum class Direction { IN, OUT }

@Serializable
enum class Status {
    WAITING, TRANSFERRING, DONE, DECLINED, CANCELLED, FAILED;

    val isFinal get() = this != WAITING && this != TRANSFERRING
}

@Serializable
enum class FileStatus { QUEUED, TRANSFERRING, DONE, FAILED, SKIPPED }

@Serializable
data class TFile(
    val id: String,
    val name: String,
    val size: Long,
    val mime: String,
    val done: Long = 0,
    val status: FileStatus = FileStatus.QUEUED,
    /** content:// of the source (sending) or of the saved file (received). */
    val uri: String? = null,
    /** The sender's small preview (a data URL) while a request waits; never saved. */
    @kotlinx.serialization.Transient val preview: String? = null,
)

@Serializable
data class Transfer(
    val id: String,
    val direction: Direction,
    val peerId: String,
    val peerAlias: String,
    val peerType: String,
    val peerModel: String? = null,
    val status: Status = Status.WAITING,
    val files: List<TFile>,
    val message: String? = null,
    val error: String? = null,
    val created: Long = System.currentTimeMillis(),
    val started: Long? = null,
    val finished: Long? = null,
    /** Accepted without asking because it came from one of your trusted devices. */
    @kotlinx.serialization.Transient val autoAccepted: Boolean = false,
    /** The sender proved its id with its TLS certificate; only then can it count as one of your devices. */
    @kotlinx.serialization.Transient val peerVerified: Boolean = false,
) {
    val total get() = files.filter { it.status != FileStatus.SKIPPED }.sumOf { it.size }
    val done get() = files.filter { it.status != FileStatus.SKIPPED }.sumOf { it.done }
    val progress get() = if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else if (status == Status.DONE) 1f else 0f
    val speed: Long
        get() {
            val start = started ?: return 0
            val elapsed = ((finished ?: System.currentTimeMillis()) - start) / 1000.0
            return if (elapsed > 0.2) (done / elapsed).toLong() else 0
        }
}

sealed interface EngineEvent {
    data class Incoming(val transferId: String) : EngineEvent
    data class Message(val transferId: String, val from: String, val text: String, val copied: Boolean) : EngineEvent
    data class Finished(val transfer: Transfer) : EngineEvent
}

/** Runtime-only state of a transfer (never shown, never persisted). */
private class Session(
    val peerIp: String,
    val sessionId: String? = null,
    val tokens: MutableMap<String, String> = ConcurrentHashMap(),
    var remoteSessionId: String? = null,
    var decision: CompletableDeferred<Boolean>? = null,
    var job: Job? = null,
    @Volatile var lastEmit: Long = 0,
)

class Engine(private val platform: Platform) {
    val settings = SettingsStore(platform.filesDir, platform.defaultAlias)
    val identity: Identity = Identity.loadOrCreate(File(platform.filesDir, "identity"))
    val client = PeerClient(identity)
    private val storage = platform.files
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var server: HttpServer? = null
    var discovery: Discovery? = null
        private set

    private val _peers = MutableStateFlow<List<Peer>>(emptyList())
    val peers: StateFlow<List<Peer>> = _peers.asStateFlow()
    private val _transfers = MutableStateFlow(loadHistory())
    val transfers: StateFlow<List<Transfer>> = _transfers.asStateFlow()
    private val _events = MutableSharedFlow<EngineEvent>(extraBufferCapacity = 64)
    val events: SharedFlow<EngineEvent> = _events.asSharedFlow()
    private val _running = MutableStateFlow(false)
    val running: StateFlow<Boolean> = _running.asStateFlow()

    private val sessions = ConcurrentHashMap<String, Session>()
    /** Wrong PINs: a few per address and a cap for everyone together, so many addresses do not help. */
    private val pinFailures = RateLimit(max = 5, maxTotal = 20, windowMs = 10 * 60_000)
    /** Requests that reach the user (prompts, messages), per sender. */
    private val requests = RateLimit(max = 20, maxTotal = 120, windowMs = 60_000)
    /** PINs the user typed for devices that require one. */
    val pins = ConcurrentHashMap<String, String>()
    var port = Protocol.DEFAULT_PORT
        private set

    fun me() = DeviceInfo(
        alias = settings.value.alias,
        version = Protocol.VERSION,
        deviceModel = platform.deviceModel,
        deviceType = "mobile",
        fingerprint = identity.fingerprint,
        port = port,
        protocol = "https",
        download = false,
    )

    // ------------------------------------------------------------- lifecycle

    @Synchronized
    fun start() {
        if (_running.value) return
        val http = HttpServer(identity.serverContext, ::handle)
        port = http.start(settings.value.port)
        server = http
        discovery = Discovery(platform, this).also { it.start(scope) }
        _running.value = true
        Log.i(TAG, "AirBridge visible as ${settings.value.alias} on port $port")
    }

    @Synchronized
    fun stop() {
        if (!_running.value) return
        discovery?.stop()
        discovery = null
        server?.stop()
        server = null
        _peers.value = emptyList()
        _running.value = false
    }

    fun refresh() {
        val d = discovery ?: return
        scope.launch { d.announceBurst() }
    }

    suspend fun scan(): Int = discovery?.scan() ?: 0

    // ----------------------------------------------------------------- peers

    /**
     * Adds or refreshes a device. A packet claiming a known device's id may not
     * take it over: no unverified update of a verified entry, no switch from
     * HTTPS to plain HTTP (which drops the certificate pin), your own devices
     * only over HTTPS, and nothing on this phone itself (another app could
     * pose as a device there).
     */
    fun upsertPeer(info: DeviceInfo, ip: String, verified: Boolean) {
        if (isLoopback(ip) && !platform.allowsLoopbackPeers) return
        val now = System.currentTimeMillis()
        _peers.update { list ->
            val index = list.indexOfFirst { it.id == info.fingerprint }
            val existing = list.getOrNull(index)
            if (!verified) {
                val downgrade = !info.isHttps && (isTrusted(info.fingerprint) || existing?.https == true)
                if (downgrade || existing?.verified == true) return@update list
            }
            val peer = Peer(
                info.fingerprint, info.alias, info.deviceModel, info.deviceType ?: "desktop", ip, info.port, info.isHttps, now,
                verified = verified || existing?.verified == true,
            )
            when {
                index >= 0 -> list.toMutableList().also { it[index] = peer }
                list.size < MAX_PEERS -> list + peer
                else -> {
                    // Full (a flood of fake devices?): drop the oldest one you don't trust.
                    val oldest = list.filter { !isTrusted(it.id) }.minByOrNull { it.lastSeen } ?: return@update list
                    list - oldest + peer
                }
            }
        }
    }

    private fun isLoopback(ip: String) =
        runCatching { java.net.InetAddress.getByName(ip).isLoopbackAddress }.getOrDefault(false)

    fun peerFailed(id: String) {
        _peers.update { list ->
            list.mapNotNull { p -> if (p.id != id) p else if (p.failures + 1 >= 2) null else p.copy(failures = p.failures + 1) }
        }
    }

    fun isTrusted(id: String) = id in settings.value.trusted

    fun setTrusted(id: String, trusted: Boolean) =
        settings.update { it.copy(trusted = if (trusted) it.trusted + id else it.trusted - id) }

    // ------------------------------------------------------------- transfers

    private fun add(transfer: Transfer) = _transfers.update { (it + transfer).takeLast(200) }

    private fun change(id: String, block: (Transfer) -> Transfer) =
        _transfers.update { list -> list.map { if (it.id == id) block(it) else it } }

    fun transfer(id: String) = _transfers.value.firstOrNull { it.id == id }

    private fun updateFile(id: String, fileId: String, block: (TFile) -> TFile) =
        change(id) { t -> t.copy(files = t.files.map { if (it.id == fileId) block(it) else it }) }

    /** Progress updates are throttled so the UI recomposes ~10×/s, not per chunk. */
    private fun progress(id: String, fileId: String, bytes: Long) {
        val session = sessions[id] ?: return
        val now = System.currentTimeMillis()
        if (now - session.lastEmit < 100) return
        session.lastEmit = now
        updateFile(id, fileId) { it.copy(done = bytes, status = FileStatus.TRANSFERRING) }
    }

    fun finish(id: String, status: Status, error: String? = null) {
        var finished: Transfer? = null
        _transfers.update { list ->
            list.map { t ->
                if (t.id != id || t.status.isFinal) t
                else t.copy(
                    status = status,
                    error = error,
                    finished = System.currentTimeMillis(),
                    // Previews only matter while a request waits.
                    files = t.files.map { it.copy(preview = null) },
                ).also { finished = it }
            }
        }
        sessions.remove(id)?.decision?.cancel()
        finished?.let {
            _events.tryEmit(EngineEvent.Finished(it))
            saveHistory()
        }
    }

    fun decide(id: String, accept: Boolean, trust: Boolean = false) {
        val t = transfer(id) ?: return
        if (accept && trust) setTrusted(t.peerId, true)
        sessions[id]?.decision?.complete(accept)
    }

    fun cancel(id: String) {
        val session = sessions[id]
        if (session?.decision?.isActive == true) {
            session.decision?.complete(false)
            return
        }
        session?.job?.cancel()
        finish(id, Status.CANCELLED, "Cancelled")
    }

    /** Fills the transfer list, and shows the phone as visible, without any network activity (screenshot tests, previews). */
    internal fun seedForPreview(list: List<Transfer>) {
        _transfers.value = list
        _running.value = true
    }

    fun clearHistory() {
        _transfers.update { list -> list.filter { !it.status.isFinal } }
        saveHistory()
    }

    // ------------------------------------------------------------ HTTP server

    private suspend fun handle(req: HttpRequest): HttpResponse {
        val api = Protocol.API
        return when {
            req.method == "POST" && req.path == "$api/register" -> register(req)
            req.method == "GET" && (req.path == "$api/info" || req.path == "/api/localsend/v1/info") -> info()
            req.method == "POST" && req.path == "$api/prepare-upload" -> prepareUpload(req)
            req.method == "POST" && req.path == "$api/upload" -> upload(req)
            req.method == "POST" && req.path == "$api/cancel" -> cancelRequest(req)
            req.path == "/" -> HttpResponse(200, "AirBridge (LocalSend protocol v2)\n".toByteArray(), "text/plain")
            else -> HttpResponse.error(404, "Not found")
        }
    }

    private fun infoJson(): String {
        val me = me()
        return Protocol.json.encodeToString(InfoDto.serializer(), InfoDto(me.alias, me.version, me.deviceModel, me.deviceType, me.fingerprint, false))
    }

    private fun info() = HttpResponse.json(200, infoJson())

    /**
     * The sender's info and whether its id is proven by its TLS client
     * certificate (the official LocalSend apps send none). Without one, the id
     * of one of your devices, or of any device that proved its id before,
     * cannot be claimed: it would show up as that device.
     */
    private fun verifiedInfo(req: HttpRequest, info: DeviceInfo): Pair<DeviceInfo, Boolean>? {
        val clean = info.sanitized() ?: return null
        val certFp = req.clientFingerprint
        if (certFp != null && clean.fingerprint.isNotEmpty() && !clean.fingerprint.equals(certFp, true)) return null
        val fp = certFp ?: clean.fingerprint
        if (!Protocol.isPeerId(fp) || fp.equals(identity.fingerprint, true)) return null
        val verified = certFp != null
        if (!verified && (isTrusted(fp) || _peers.value.any { it.id == fp && it.verified })) {
            Log.w(TAG, "${req.remoteIp} claims to be ${clean.alias} without its certificate")
            return null
        }
        return clean.copy(fingerprint = fp) to verified
    }

    private fun register(req: HttpRequest): HttpResponse {
        val info = runCatching { Protocol.json.decodeFromString(DeviceInfo.serializer(), String(req.readBody(1_000_000))) }.getOrNull()
            ?: return HttpResponse.error(400, "Invalid body")
        verifiedInfo(req, info)?.let { (clean, verified) -> upsertPeer(clean, req.remoteIp, verified) }
        return info()
    }

    private fun checkPin(req: HttpRequest): HttpResponse? {
        val pin = settings.value.pin
        if (pin.isEmpty()) return null
        if (!pinFailures.allows(req.remoteIp)) return HttpResponse.error(429, "Too many requests")
        val given = req.query["pin"] ?: return HttpResponse.error(401, "PIN required")
        // Digests: equal length, so the comparison reveals nothing about the PIN.
        val sha = MessageDigest.getInstance("SHA-256")
        if (!MessageDigest.isEqual(sha.digest(given.toByteArray()), sha.digest(pin.toByteArray()))) {
            pinFailures.record(req.remoteIp)
            return HttpResponse.error(401, "Invalid PIN")
        }
        return null
    }

    private suspend fun prepareUpload(req: HttpRequest): HttpResponse {
        checkPin(req)?.let { return it }
        val body = runCatching {
            Protocol.json.decodeFromString(PrepareUploadRequest.serializer(), String(req.readBody()))
        }.getOrNull() ?: return HttpResponse.error(400, "Invalid body")
        val (info, verified) = verifiedInfo(req, body.info) ?: return HttpResponse.error(400, "Invalid sender")
        if (body.files.size > Protocol.MAX_FILES) return HttpResponse.error(413, "Too many files")
        val files = body.files.values.mapNotNull { it.sanitized() }
        if (files.isEmpty() || files.size != body.files.size) return HttpResponse.error(400, "Invalid files")
        if (files.map { it.id }.toSet().size != files.size) return HttpResponse.error(400, "Duplicate file ids")
        runCatching { files.fold(0L) { total, f -> Math.addExact(total, f.size) } }.getOrNull()
            ?: return HttpResponse.error(400, "Invalid file sizes")
        // Every request reaches the user (a prompt or a message): not too many.
        if (!requests.hit(req.remoteIp)) return HttpResponse.error(429, "Too many requests")
        upsertPeer(info, req.remoteIp, verified)
        // Only your own devices, proven by their certificate, skip the question.
        val autoAccept = settings.value.autoAcceptTrusted && verified && isTrusted(info.fingerprint)

        val base = Transfer(
            id = UUID.randomUUID().toString().take(12),
            direction = Direction.IN,
            peerId = info.fingerprint,
            peerAlias = info.alias,
            peerType = info.deviceType ?: "desktop",
            peerModel = info.deviceModel,
            files = files.map { TFile(it.id, it.fileName, it.size, it.fileType, preview = Previews.accept(it.preview)) },
            autoAccepted = autoAccept,
            peerVerified = verified,
        )

        val text = files.singleOrNull()?.takeIf { it.isTextMessage }?.preview
        if (text != null) {
            // Text is shown, not saved: no confirmation needed.
            add(base.copy(message = text, started = System.currentTimeMillis(), files = base.files.map { it.copy(done = it.size, status = FileStatus.DONE) }))
            finish(base.id, Status.DONE)
            onTextReceived(base.id, info.alias, info.fingerprint, text, verified)
            return HttpResponse.empty(204)
        }

        // The decision handle exists before the request becomes visible, so an
        // answer can never arrive "too early".
        val decision = if (autoAccept) null else CompletableDeferred<Boolean>()
        val session = Session(peerIp = req.remoteIp, sessionId = UUID.randomUUID().toString(), decision = decision)
        synchronized(this) {
            val busy = _transfers.value.any { it.direction == Direction.IN && !it.status.isFinal && sessions[it.id]?.sessionId != null }
            if (busy) return HttpResponse.error(409, "Blocked by another session")
            sessions[base.id] = session
            add(base)
        }

        val accepted = if (decision == null) {
            true
        } else {
            req.onDisconnect { if (decision.isActive) finish(base.id, Status.CANCELLED, "The sender cancelled") }
            _events.tryEmit(EngineEvent.Incoming(base.id))
            withTimeoutOrNull(5 * 60_000) { runCatching { decision.await() }.getOrDefault(false) } ?: false
        }
        session.decision = null
        if (transfer(base.id)?.status?.isFinal != false) return HttpResponse.error(403, "Cancelled")
        if (!accepted) {
            finish(base.id, Status.DECLINED)
            return HttpResponse.error(403, "Rejected")
        }
        if (base.total > storage.freeBytes()) {
            finish(base.id, Status.FAILED, "Not enough storage")
            return HttpResponse.error(500, "Not enough storage on the receiver")
        }
        files.forEach { session.tokens[it.id] = UUID.randomUUID().toString().replace("-", "") }
        change(base.id) { it.copy(status = Status.TRANSFERRING, started = System.currentTimeMillis()) }
        val response = PrepareUploadResponse(session.sessionId!!, session.tokens.toMap())
        return HttpResponse.json(200, Protocol.json.encodeToString(PrepareUploadResponse.serializer(), response))
    }

    private suspend fun upload(req: HttpRequest): HttpResponse {
        val sessionId = req.query["sessionId"]
        val fileId = req.query["fileId"]
        val token = req.query["token"]
        if (sessionId == null || fileId == null || token == null) return HttpResponse.error(400, "Missing parameters")
        val entry = sessions.entries.firstOrNull { it.value.sessionId == sessionId } ?: return HttpResponse.error(403, "Invalid session")
        val (id, session) = entry.key to entry.value
        if (session.peerIp != req.remoteIp) return HttpResponse.error(403, "Invalid IP address")
        val expected = session.tokens[fileId]
        if (expected == null || !MessageDigest.isEqual(expected.toByteArray(), token.toByteArray())) return HttpResponse.error(403, "Invalid token")
        val transfer = transfer(id) ?: return HttpResponse.error(403, "Invalid session")
        val file = transfer.files.firstOrNull { it.id == fileId } ?: return HttpResponse.error(403, "Invalid file")
        if (file.status == FileStatus.DONE) return HttpResponse.error(409, "Already received")
        if (file.status == FileStatus.TRANSFERRING) return HttpResponse.error(409, "Already receiving")
        val declared = req.headers["content-length"]?.toLongOrNull()
        if (declared != null && declared > file.size) return HttpResponse.error(413, "More data than announced")

        return withContext(Dispatchers.IO) {
            var pending: FileStore.Pending? = null
            try {
                pending = storage.create(file.name, file.mime)
                updateFile(id, fileId) { it.copy(status = FileStatus.TRANSFERRING, done = 0) }
                val buffer = ByteArray(256 * 1024)
                var received = 0L
                while (true) {
                    val n = req.body.read(buffer)
                    if (n < 0) break
                    // Never more than was announced (and accepted).
                    if (received + n > file.size) throw IOException("more than the announced ${file.size} bytes")
                    pending.output.write(buffer, 0, n)
                    received += n
                    if (transfer(id)?.status?.isFinal != false) throw CancelledByUser()
                    progress(id, fileId, received)
                }
                if (received != file.size) throw IOException("expected ${file.size} bytes, got $received")
                storage.publish(pending)
                updateFile(id, fileId) { it.copy(status = FileStatus.DONE, done = it.size, uri = pending.uri) }
                finishIfComplete(id)
                HttpResponse.empty(200)
            } catch (e: CancelledByUser) {
                pending?.let(storage::discard)
                HttpResponse.error(409, "Session cancelled")
            } catch (e: Exception) {
                Log.w(TAG, "receiving ${file.name} failed", e)
                pending?.let(storage::discard)
                updateFile(id, fileId) { it.copy(status = FileStatus.FAILED) }
                finishIfComplete(id)
                HttpResponse.error(500, "Could not save the file")
            }
        }
    }

    private class CancelledByUser : Exception()

    private fun finishIfComplete(id: String) {
        val t = transfer(id) ?: return
        if (t.files.any { it.status == FileStatus.QUEUED || it.status == FileStatus.TRANSFERRING }) return
        if (t.files.all { it.status == FileStatus.DONE || it.status == FileStatus.SKIPPED }) finish(id, Status.DONE)
        else finish(id, Status.FAILED, "Some files could not be received")
    }

    private fun cancelRequest(req: HttpRequest): HttpResponse {
        val sessionId = req.query["sessionId"] ?: return HttpResponse.error(400, "Missing parameters")
        sessions.entries.firstOrNull { it.value.sessionId == sessionId && it.value.peerIp == req.remoteIp }?.let {
            finish(it.key, Status.CANCELLED, "The sender cancelled")
            return HttpResponse.empty(200)
        }
        // The receiver of one of our transfers cancelled it.
        sessions.entries.firstOrNull { it.value.remoteSessionId == sessionId && it.value.peerIp == req.remoteIp }?.let {
            it.value.job?.cancel()
            finish(it.key, Status.CANCELLED, "The receiver cancelled")
        }
        return HttpResponse.empty(200)
    }

    private fun onTextReceived(id: String, from: String, peerId: String, text: String, verified: Boolean) {
        // Only your own devices, proven by their certificate, write to the clipboard.
        val copy = settings.value.copyReceivedText && verified && isTrusted(peerId)
        if (copy) platform.copyToClipboard(text)
        _events.tryEmit(EngineEvent.Message(id, from, text, copy))
    }

    fun copyToClipboard(text: String) = platform.copyToClipboard(text)

    // ---------------------------------------------------------------- sending

    fun describe(uris: List<String>): List<FileStore.Picked> = uris.mapNotNull(storage::describe)

    fun sendFiles(peer: Peer, uris: List<String>): String? {
        val picked = uris.mapNotNull(storage::describe)
        if (picked.isEmpty()) return null
        val files = picked.map { TFile(UUID.randomUUID().toString(), it.name, it.size, it.mime, uri = it.uri) }
        return startSend(peer, Transfer(UUID.randomUUID().toString().take(12), Direction.OUT, peer.id, peer.alias, peer.deviceType, peer.deviceModel, files = files))
    }

    fun sendText(peer: Peer, text: String): String {
        val dto = FileDto.forText(text)
        val file = TFile(dto.id, dto.fileName, dto.size, dto.fileType)
        return startSend(peer, Transfer(UUID.randomUUID().toString().take(12), Direction.OUT, peer.id, peer.alias, peer.deviceType, peer.deviceModel, files = listOf(file), message = text))
    }

    private fun startSend(peer: Peer, transfer: Transfer): String {
        val session = Session(peerIp = peer.ip)
        sessions[transfer.id] = session
        add(transfer)
        session.job = scope.launch {
            try {
                runSend(peer, transfer.id, session)
            } catch (e: kotlinx.coroutines.CancellationException) {
                session.remoteSessionId?.let { remote ->
                    CoroutineScope(Dispatchers.IO).launch {
                        runCatching { client.post(peer.ip, peer.port, peer.https, peer.id, "${Protocol.API}/cancel", mapOf("sessionId" to remote), readTimeoutMs = 5_000) }
                    }
                }
                finish(transfer.id, Status.CANCELLED, "Cancelled")
            } catch (e: PinMismatchException) {
                finish(transfer.id, Status.FAILED, e.message)
            } catch (e: IOException) {
                finish(transfer.id, Status.FAILED, "Can't reach ${peer.alias}: ${e.message ?: "connection failed"}")
            } catch (e: Exception) {
                Log.w(TAG, "send failed", e)
                finish(transfer.id, Status.FAILED, e.message ?: "Unexpected error")
            }
        }
        return transfer.id
    }

    private suspend fun runSend(peer: Peer, id: String, session: Session) {
        val transfer = transfer(id) ?: return
        // The first few photos travel with a small preview, so the other side
        // sees what is coming before it accepts.
        val previews = withContext(Dispatchers.IO) {
            transfer.files.filter { transfer.message == null && it.uri != null && Previews.isPreviewable(it.mime) }
                .take(Previews.MAX_PREVIEWS)
                .mapNotNull { f -> runCatching { platform.thumbnail(f.uri!!, f.mime) }.getOrNull()?.let { f.id to Previews.dataUrl(it) } }
                .toMap()
        }
        val dtos = transfer.files.associate { f ->
            f.id to (if (transfer.message != null) FileDto.forText(transfer.message).copy(id = f.id, fileName = f.name)
            else FileDto(f.id, f.name, f.size, f.mime, preview = previews[f.id]))
        }
        val body = Protocol.json.encodeToString(PrepareUploadRequest.serializer(), PrepareUploadRequest(me(), dtos))
        // A PIN only travels encrypted, to a certificate-pinned device.
        val pin = pins[peer.id]?.takeIf { peer.https }
        val result = client.post(
            peer.ip, peer.port, peer.https, peer.id, "${Protocol.API}/prepare-upload",
            query = if (pin.isNullOrEmpty()) emptyMap() else mapOf("pin" to pin),
            body = PeerClient.json(body),
            readTimeoutMs = 0, // waits until the other side accepts
            connectTimeoutMs = 6_000,
        )
        when (result.status) {
            200 -> Unit
            204 -> {
                change(id) { t -> t.copy(started = t.started ?: System.currentTimeMillis(), files = t.files.map { it.copy(done = it.size, status = FileStatus.DONE) }) }
                return finish(id, Status.DONE)
            }
            403 -> return finish(id, Status.DECLINED, "Declined")
            401 -> return finish(id, Status.FAILED, if (pin.isNullOrEmpty()) "PIN required" else "Wrong PIN")
            409 -> return finish(id, Status.FAILED, "The device is busy with another transfer")
            429 -> return finish(id, Status.FAILED, "Too many attempts, try again later")
            else -> return finish(id, Status.FAILED, "Refused (HTTP ${result.status})")
        }
        val accepted = Protocol.json.decodeFromString(PrepareUploadResponse.serializer(), result.body)
        session.remoteSessionId = accepted.sessionId
        change(id) { t ->
            t.copy(
                status = Status.TRANSFERRING,
                started = System.currentTimeMillis(),
                files = t.files.map { if (it.id in accepted.files) it else it.copy(status = FileStatus.SKIPPED) },
            )
        }
        for (file in transfer.files) {
            val token = accepted.files[file.id] ?: continue
            updateFile(id, file.id) { it.copy(status = FileStatus.TRANSFERRING) }
            val message = transfer.message
            val upload = StreamingBody(
                length = file.size,
                mime = file.mime,
                open = { if (message != null) message.toByteArray().inputStream() else storage.open(file.uri ?: throw IOException("no data")) },
                onProgress = { progress(id, file.id, it) },
            )
            val response = client.post(
                peer.ip, peer.port, peer.https, peer.id, "${Protocol.API}/upload",
                query = mapOf("sessionId" to accepted.sessionId, "fileId" to file.id, "token" to token),
                body = upload,
                readTimeoutMs = 120_000,
            )
            if (response.status != 200 && response.status != 204) {
                updateFile(id, file.id) { it.copy(status = FileStatus.FAILED) }
                return finish(id, Status.FAILED, if (response.status == 403 || response.status == 409) "The receiver cancelled" else "Upload refused (HTTP ${response.status})")
            }
            updateFile(id, file.id) { it.copy(status = FileStatus.DONE, done = it.size) }
        }
        finish(id, Status.DONE)
    }

    // ---------------------------------------------------------------- history

    private val historyFile get() = File(platform.filesDir, "history.json")

    private fun loadHistory(): List<Transfer> = runCatching {
        Protocol.json.decodeFromString(ListSerializer(Transfer.serializer()), File(platform.filesDir, "history.json").readText())
            .map { if (it.status.isFinal) it else it.copy(status = Status.FAILED, error = "Interrupted") }
    }.getOrDefault(emptyList())

    private fun saveHistory() {
        val finished = _transfers.value.filter { it.status.isFinal }.takeLast(100)
        scope.launch(Dispatchers.IO) {
            runCatching {
                val tmp = File(historyFile.path + ".tmp")
                tmp.writeText(Protocol.json.encodeToString(ListSerializer(Transfer.serializer()), finished))
                tmp.renameTo(historyFile)
            }
        }
    }

    fun close() {
        stop()
        scope.cancel()
    }

    companion object {
        private const val TAG = "Engine"
        /** Most devices kept at once; a flood of fake ones evicts the oldest untrusted. */
        private const val MAX_PEERS = 64
    }
}
