package io.github.mdhaarith.airbridge.core

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.net.URLDecoder
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLServerSocket
import javax.net.ssl.SSLSocket

class HttpRequest(
    val method: String,
    val path: String,
    val query: Map<String, String>,
    val headers: Map<String, String>,
    val body: InputStream,
    val remoteIp: String,
    /** Fingerprint of the TLS client certificate, if the client sent one. */
    val clientFingerprint: String?,
    private val watcher: (() -> Unit) -> Unit,
) {
    fun readBody(limit: Int = 32 * 1024 * 1024): ByteArray {
        val out = ByteArrayOutputStream()
        val buffer = ByteArray(16 * 1024)
        while (true) {
            val n = body.read(buffer)
            if (n < 0) break
            out.write(buffer, 0, n)
            if (out.size() > limit) throw IOException("body too large")
        }
        return out.toByteArray()
    }

    /** Calls [onGone] if the client hangs up while we are still deciding. */
    fun onDisconnect(onGone: () -> Unit) = watcher(onGone)
}

class HttpResponse(val status: Int, val body: ByteArray = ByteArray(0), val contentType: String = "application/json") {
    companion object {
        fun json(status: Int, json: String) = HttpResponse(status, json.toByteArray())
        fun error(status: Int, message: String) =
            json(status, Protocol.json.encodeToString(ErrorDto.serializer(), ErrorDto(message)))
        fun empty(status: Int) = HttpResponse(status)
    }
}

/**
 * A deliberately small HTTP/1.1 server: the LocalSend API has five routes, but
 * needs TLS client certificates, streaming uploads and chunked bodies.
 */
class HttpServer(
    private val ssl: SSLContext?,
    private val handler: suspend (HttpRequest) -> HttpResponse,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var server: ServerSocket? = null
    var port = 0
        private set

    fun start(preferredPort: Int): Int {
        var lastError: IOException? = null
        for (candidate in preferredPort until preferredPort + 10) {
            try {
                val socket = if (ssl != null) {
                    (ssl.serverSocketFactory.createServerSocket() as SSLServerSocket).apply { wantClientAuth = true }
                } else {
                    ServerSocket()
                }
                socket.reuseAddress = true
                socket.bind(InetSocketAddress(candidate))
                server = socket
                port = candidate
                scope.launch { acceptLoop(socket) }
                return candidate
            } catch (e: IOException) {
                lastError = e
            }
        }
        throw lastError ?: IOException("no free port")
    }

    fun stop() {
        runCatching { server?.close() }
        scope.cancel()
    }

    private suspend fun acceptLoop(socket: ServerSocket) {
        while (scope.isActive && !socket.isClosed) {
            val client = try {
                socket.accept()
            } catch (e: IOException) {
                if (socket.isClosed) return
                continue
            }
            scope.launch { serve(client) }
        }
    }

    private suspend fun serve(socket: Socket) {
        try {
            socket.soTimeout = 60_000
            var fingerprint: String? = null
            if (socket is SSLSocket) {
                socket.startHandshake()
                fingerprint = runCatching { (socket.session.peerCertificates.first() as X509Certificate).let(Identity::fingerprintOf) }.getOrNull()
            }
            val input = BufferedInputStream(socket.getInputStream(), 64 * 1024)
            val output = BufferedOutputStream(socket.getOutputStream(), 64 * 1024)
            val remoteIp = socket.inetAddress.hostAddress?.removePrefix("::ffff:") ?: ""
            while (true) {
                val requestLine = readLine(input) ?: break
                if (requestLine.isBlank()) continue
                val parts = requestLine.split(" ")
                if (parts.size < 3) break
                val headers = mutableMapOf<String, String>()
                while (true) {
                    val line = readLine(input) ?: return
                    if (line.isEmpty()) break
                    val colon = line.indexOf(':')
                    if (colon > 0) headers[line.substring(0, colon).trim().lowercase()] = line.substring(colon + 1).trim()
                }
                val (path, query) = parseTarget(parts[1])
                val body: InputStream = when {
                    headers["transfer-encoding"]?.contains("chunked", true) == true -> ChunkedInputStream(input)
                    headers["content-length"] != null -> BoundedInputStream(input, headers["content-length"]!!.toLong())
                    else -> BoundedInputStream(input, 0)
                }
                if (headers["expect"].equals("100-continue", true)) {
                    output.write("HTTP/1.1 100 Continue\r\n\r\n".toByteArray())
                    output.flush()
                }
                var closeAfter = parts[2] == "HTTP/1.0" || headers["connection"].equals("close", true)
                val request = HttpRequest(parts[0], path, query, headers, body, remoteIp, fingerprint) { onGone ->
                    // The client sends nothing while waiting for our answer: reading
                    // EOF means it hung up. The connection closes after the reply.
                    closeAfter = true
                    Thread {
                        try {
                            socket.soTimeout = 0
                            if (input.read() < 0) onGone()
                        } catch (_: IOException) {
                        }
                    }.apply { isDaemon = true }.start()
                }
                val response = try {
                    handler(request)
                } catch (e: Exception) {
                    Log.w(TAG, "handler failed", e)
                    HttpResponse.error(500, "Internal error")
                }
                // Leftover body we did not read (e.g. a rejected upload): close instead of draining gigabytes.
                if (body is BoundedInputStream && body.remaining > 1_000_000) closeAfter = true
                if (!closeAfter) runCatching { while (body.read(ByteArray(8192)) >= 0) Unit }
                write(output, response, closeAfter)
                if (closeAfter) break
            }
        } catch (_: SocketTimeoutException) {
        } catch (e: IOException) {
            Log.d(TAG, "connection ended: ${e.message}")
        } finally {
            runCatching { socket.close() }
        }
    }

    private fun write(out: BufferedOutputStream, response: HttpResponse, close: Boolean) {
        val head = buildString {
            append("HTTP/1.1 ${response.status} ${reason(response.status)}\r\n")
            if (response.body.isNotEmpty()) append("Content-Type: ${response.contentType}\r\n")
            append("Content-Length: ${response.body.size}\r\n")
            append(if (close) "Connection: close\r\n" else "Connection: keep-alive\r\n")
            append("\r\n")
        }
        out.write(head.toByteArray(Charsets.ISO_8859_1))
        out.write(response.body)
        out.flush()
    }

    companion object {
        private const val TAG = "HttpServer"

        private fun reason(status: Int) = when (status) {
            200 -> "OK"; 204 -> "No Content"; 400 -> "Bad Request"; 401 -> "Unauthorized"; 403 -> "Forbidden"
            404 -> "Not Found"; 409 -> "Conflict"; 422 -> "Unprocessable Entity"; 429 -> "Too Many Requests"
            else -> "Status"
        }

        fun readLine(input: InputStream): String? {
            val bytes = ByteArrayOutputStream()
            while (true) {
                val b = input.read()
                if (b < 0) return if (bytes.size() == 0) null else bytes.toString(Charsets.ISO_8859_1.name())
                if (b == '\n'.code) break
                if (b != '\r'.code) bytes.write(b)
                if (bytes.size() > 16 * 1024) throw IOException("line too long")
            }
            return bytes.toString(Charsets.ISO_8859_1.name())
        }

        private fun parseTarget(target: String): Pair<String, Map<String, String>> {
            val q = target.indexOf('?')
            if (q < 0) return target to emptyMap()
            val query = target.substring(q + 1).split('&').filter { it.isNotEmpty() }.associate { pair ->
                val eq = pair.indexOf('=')
                val key = if (eq < 0) pair else pair.substring(0, eq)
                val value = if (eq < 0) "" else pair.substring(eq + 1)
                URLDecoder.decode(key, "UTF-8") to URLDecoder.decode(value, "UTF-8")
            }
            return target.substring(0, q) to query
        }
    }
}

class BoundedInputStream(private val inner: InputStream, length: Long) : InputStream() {
    var remaining = length
        private set

    override fun read(): Int {
        if (remaining <= 0) return -1
        val b = inner.read()
        if (b >= 0) remaining-- else throw IOException("unexpected end of body")
        return b
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (remaining <= 0) return -1
        val n = inner.read(b, off, minOf(len.toLong(), remaining).toInt())
        if (n < 0) throw IOException("unexpected end of body")
        remaining -= n
        return n
    }
}

class ChunkedInputStream(private val inner: InputStream) : InputStream() {
    private var chunkLeft = 0L
    private var finished = false

    private fun nextChunk(): Boolean {
        if (finished) return false
        val line = HttpServer.readLine(inner) ?: throw IOException("unexpected end of chunked body")
        val size = line.substringBefore(';').trim().ifEmpty { "0" }.toLong(16)
        if (size == 0L) {
            while ((HttpServer.readLine(inner) ?: "").isNotEmpty()) Unit // trailers
            finished = true
            return false
        }
        chunkLeft = size
        return true
    }

    override fun read(): Int {
        val one = ByteArray(1)
        return if (read(one, 0, 1) < 0) -1 else one[0].toInt() and 0xff
    }

    override fun read(b: ByteArray, off: Int, len: Int): Int {
        if (chunkLeft == 0L && !nextChunk()) return -1
        val n = inner.read(b, off, minOf(len.toLong(), chunkLeft).toInt())
        if (n < 0) throw IOException("unexpected end of chunk")
        chunkLeft -= n
        if (chunkLeft == 0L) HttpServer.readLine(inner) // CRLF after the chunk
        return n
    }
}
