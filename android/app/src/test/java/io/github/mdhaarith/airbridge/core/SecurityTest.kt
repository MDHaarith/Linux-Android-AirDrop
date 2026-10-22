package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.net.Socket
import java.security.cert.X509Certificate
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/** What a stranger on the same Wi‑Fi can (not) do to the phone. */
class SecurityTest {
    @get:Rule
    val tmp = TemporaryFolder()

    /** Sends one raw HTTP request and returns the status code. */
    private fun rawStatus(socket: Socket, request: String): Int {
        socket.soTimeout = 10_000
        socket.getOutputStream().apply { write(request.toByteArray()); flush() }
        val line = socket.getInputStream().bufferedReader().readLine() ?: return -1
        return line.split(" ")[1].toInt()
    }

    /** A TLS connection that presents no client certificate, like the official LocalSend apps. */
    private fun tlsWithoutCertificate(port: Int): SSLSocket {
        val trustAll = object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) = Unit
            override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) = Unit
            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
        }
        val context = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trustAll), null) }
        return (context.socketFactory.createSocket("127.0.0.1", port) as SSLSocket).apply { startHandshake() }
    }

    @Test
    fun malformedRequestsAreAnsweredNotFatal() {
        val http = HttpServer(null) { HttpResponse.json(200, "{}") }
        val port = http.start(46_300)
        try {
            Socket("127.0.0.1", port).use { assertEquals(400, rawStatus(it, "GET /?a=%zz HTTP/1.1\r\nHost: t\r\n\r\n")) }
            Socket("127.0.0.1", port).use { assertEquals(400, rawStatus(it, "POST / HTTP/1.1\r\nContent-Length: lots\r\n\r\n")) }
            // Still serving afterwards.
            Socket("127.0.0.1", port).use { assertEquals(200, rawStatus(it, "GET / HTTP/1.1\r\nHost: t\r\n\r\n")) }
        } finally {
            http.stop()
        }
    }

    @Test
    fun aTrustedDeviceCannotBeImpersonatedWithoutItsCertificate() = runBlocking {
        val laptop = Identity.loadOrCreate(tmp.newFolder("laptop"))
        val engine = Engine(FakePlatform(tmp.newFolder("phone"), "Phone"))
        engine.settings.update { it.copy(port = 46_400, trusted = setOf(laptop.fingerprint)) }
        engine.start()
        try {
            val port = engine.port
            val offer = """{"info":{"alias":"Laptop","fingerprint":"${laptop.fingerprint}","deviceType":"desktop"},
                "files":{"f":{"id":"f","fileName":"evil.apk","size":4,"fileType":"application/vnd.android.package-archive"}}}"""
                .replace("\n", "")
            // Claiming the laptop's (public) fingerprint without its certificate: refused.
            tlsWithoutCertificate(port).use { socket ->
                val status = rawStatus(
                    socket,
                    "POST ${Protocol.API}/prepare-upload HTTP/1.1\r\nHost: t\r\nContent-Type: application/json\r\n" +
                        "Content-Length: ${offer.toByteArray().size}\r\nConnection: close\r\n\r\n$offer",
                )
                assertEquals(400, status)
            }
            assertTrue(engine.transfers.value.none { it.direction == Direction.IN })
            // The real laptop, with its certificate, is accepted without asking.
            val real = PeerClient(laptop).post(
                "127.0.0.1", port, true, engine.identity.fingerprint, "${Protocol.API}/prepare-upload",
                body = PeerClient.json(offer),
            )
            assertEquals(real.body, 200, real.status)
            assertTrue(engine.transfers.value.single { it.direction == Direction.IN }.let { it.autoAccepted && it.peerVerified })
        } finally {
            engine.close()
        }
    }
}
