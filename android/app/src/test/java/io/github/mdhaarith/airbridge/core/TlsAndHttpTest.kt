package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.net.Socket

class TlsAndHttpTest {
    @get:Rule
    val tmp = TemporaryFolder()

    @Test
    fun identityIsStableAndLocalSendShaped() {
        val dir = tmp.newFolder()
        val a = Identity.loadOrCreate(dir)
        val b = Identity.loadOrCreate(dir)
        assertEquals(a.fingerprint, b.fingerprint)
        assertTrue(a.fingerprint.matches(Regex("[0-9A-F]{64}")))
        a.certificate.verify(a.certificate.publicKey)
    }

    @Test
    fun mutualTlsWithPinning() = runBlocking {
        val server = Identity.loadOrCreate(tmp.newFolder())
        val client = Identity.loadOrCreate(tmp.newFolder())
        val http = HttpServer(server.serverContext) { req ->
            HttpResponse.json(200, """{"client":"${req.clientFingerprint}","body":${req.readBody().size}}""")
        }
        val port = http.start(46_000)
        try {
            val peer = PeerClient(client)
            val ok = peer.post("127.0.0.1", port, true, server.fingerprint, "/echo", body = PeerClient.json("{\"x\":1}"))
            assertEquals(200, ok.status)
            // The server saw our client certificate, and we saw (and pinned) its certificate.
            assertTrue(ok.body, ok.body.contains(client.fingerprint))
            assertEquals(server.fingerprint, ok.peerFingerprint)

            try {
                peer.post("127.0.0.1", port, true, "0".repeat(64), "/echo")
                fail("a wrong pin must abort the handshake")
            } catch (_: PinMismatchException) {
            }
        } finally {
            http.stop()
        }
    }

    @Test
    fun chunkedBodiesAndKeepAlive() {
        val http = HttpServer(null) { req -> HttpResponse.json(200, """{"n":${req.readBody().size},"q":"${req.query["a"]}"}""") }
        val port = http.start(46_100)
        try {
            Socket("127.0.0.1", port).use { socket ->
                val out = socket.getOutputStream()
                val input = socket.getInputStream().bufferedReader(Charsets.ISO_8859_1)
                out.write("POST /x?a=h%C3%A9 HTTP/1.1\r\nHost: t\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".toByteArray())
                out.write("POST /x HTTP/1.1\r\nHost: t\r\nContent-Length: 3\r\n\r\nabc".toByteArray())
                out.flush()
                val responses = mutableListOf<String>()
                var length = -1
                while (responses.size < 2) {
                    val line = input.readLine() ?: break
                    if (line.startsWith("Content-Length", true)) length = line.substringAfter(":").trim().toInt()
                    if (line.isEmpty() && length >= 0) {
                        val body = CharArray(length)
                        var read = 0
                        while (read < length) read += input.read(body, read, length - read)
                        responses += String(String(body).toByteArray(Charsets.ISO_8859_1), Charsets.UTF_8)
                        length = -1
                    }
                }
                assertEquals("""{"n":11,"q":"hé"}""", responses[0])
                assertEquals("""{"n":3,"q":"null"}""", responses[1])
            }
        } finally {
            http.stop()
        }
    }
}
