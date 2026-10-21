package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Protocol as HttpProtocol
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okio.BufferedSink
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class PeerResult(val status: Int, val body: String, val peerFingerprint: String?)

class PinMismatchException(message: String) : IOException(message)

/** HTTP(S) client for LocalSend peers: our certificate as client cert, the peer's pinned. */
class PeerClient(private val identity: Identity) {
    private val base = OkHttpClient.Builder()
        // Peers are on the local network: never route through a configured HTTP proxy.
        .proxy(java.net.Proxy.NO_PROXY)
        .protocols(listOf(HttpProtocol.HTTP_1_1))
        .retryOnConnectionFailure(false)
        .build()

    private fun client(expectedFingerprint: String?, https: Boolean, readTimeoutMs: Long, connectTimeoutMs: Long): Pair<OkHttpClient, PinnedTrustManager?> {
        val builder = base.newBuilder()
            .connectTimeout(connectTimeoutMs, TimeUnit.MILLISECONDS)
            .readTimeout(readTimeoutMs, TimeUnit.MILLISECONDS)
            .writeTimeout(60, TimeUnit.SECONDS)
        if (!https) return builder.build() to null
        val (context, trust) = identity.clientContext(expectedFingerprint)
        builder.sslSocketFactory(context.socketFactory, trust)
            // Peers are addressed by IP and identified by their fingerprint, not a hostname.
            .hostnameVerifier { _, _ -> true }
        return builder.build() to (trust as PinnedTrustManager)
    }

    private fun url(ip: String, port: Int, https: Boolean, path: String, query: Map<String, String>): HttpUrl =
        HttpUrl.Builder()
            .scheme(if (https) "https" else "http")
            .host(ip)
            .port(port)
            .encodedPath(path)
            .apply { query.forEach { (k, v) -> addQueryParameter(k, v) } }
            .build()

    suspend fun post(
        ip: String,
        port: Int,
        https: Boolean,
        fingerprint: String?,
        path: String,
        query: Map<String, String> = emptyMap(),
        body: RequestBody = ByteArray(0).toRequestBody(null),
        readTimeoutMs: Long = 30_000,
        connectTimeoutMs: Long = 5_000,
    ): PeerResult = withContext(Dispatchers.IO) {
        val (client, trust) = client(fingerprint?.takeIf { https }, https, readTimeoutMs, connectTimeoutMs)
        val request = Request.Builder()
            .url(url(ip, port, https, path, query))
            .header("User-Agent", "AirBridge-Android/0.1")
            .post(body)
            .build()
        val call = client.newCall(request)
        try {
            call.await().use { response ->
                PeerResult(response.code, response.body?.string().orEmpty(), trust?.lastFingerprint)
            }
        } catch (e: IOException) {
            if (generateSequence<Throwable>(e) { it.cause }.any { it.message?.contains("fingerprint mismatch") == true }) {
                throw PinMismatchException("The device's certificate does not match (possible impersonation)")
            }
            throw e
        }
    }

    companion object {
        val JSON = "application/json".toMediaType()

        fun json(text: String): RequestBody = text.toRequestBody(JSON)
    }
}

/** Streams [open] with a known length, reporting progress. */
class StreamingBody(
    private val length: Long,
    private val mime: String,
    private val open: () -> InputStream,
    private val onProgress: (Long) -> Unit,
) : RequestBody() {
    override fun contentType() = runCatching { mime.toMediaType() }.getOrNull()
    override fun contentLength() = length
    override fun isOneShot() = true
    override fun writeTo(sink: BufferedSink) {
        open().use { input ->
            val buffer = ByteArray(256 * 1024)
            var sent = 0L
            while (true) {
                val n = input.read(buffer)
                if (n < 0) break
                sink.write(buffer, 0, n)
                sent += n
                onProgress(sent)
            }
        }
    }
}

private suspend fun Call.await(): Response = suspendCancellableCoroutine { cont ->
    enqueue(object : Callback {
        override fun onResponse(call: Call, response: Response) = cont.resume(response)
        override fun onFailure(call: Call, e: IOException) {
            if (!cont.isCancelled) cont.resumeWithException(e)
        }
    })
    cont.invokeOnCancellation { runCatching { cancel() } }
}
