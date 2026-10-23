package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.net.Socket

class CompanionTest {
    @get:Rule val tmp = TemporaryFolder()

    private fun request(path: String, key: String? = null, method: String = "GET", body: String = "", query: Map<String, String> = emptyMap(), headers: Map<String, String> = emptyMap()) =
        HttpRequest(method, path, query + if (key == null) emptyMap() else mapOf("k" to key, "id" to "browser1"), headers, ByteArrayInputStream(body.toByteArray()), "127.0.0.1", null) {}

    @Test fun authorizationAndAssetTraversal() = runBlocking {
        val engine = Engine(FakePlatform(tmp.newFolder(), "Phone"))
        val web = BrowserCompanion(engine) { "asset".toByteArray() }
        for (route in listOf("/app", "/hello", "/poll", "/send", "/upload", "/text", "/ws", "/offer/a/accept", "/download/a/b", "/assets/shared.js")) {
            assertEquals(route, 403, web.handle(request(route)).status)
            assertEquals(route, 403, web.handle(request(route, "wrong")).status)
        }
        assertEquals(200, web.handle(request("/")).status)
        assertEquals(200, web.handle(request("/hello", web.secret)).status)
        assertEquals(404, web.handle(request("/assets/../identity", web.secret)).status)
        assertEquals(404, web.handle(request("/assets/%2e%2e/identity", web.secret)).status)
        assertEquals(200, web.handle(request("/assets/shared.js", web.secret)).status)
        assertFalse(BrowserCompanion.authorized(web.secret, null))
        assertTrue(BrowserCompanion.authorized(web.secret, web.secret))
        web.close()
        assertEquals(410, web.handle(request("/hello", web.secret)).status)
    }

    @Test fun rangeParsing() {
        assertEquals(0L to 10L, BrowserCompanion.byteRange(null, 10))
        assertEquals(2L to 4L, BrowserCompanion.byteRange("bytes=2-5", 10))
        assertEquals(8L to 2L, BrowserCompanion.byteRange("bytes=8-", 10))
        assertEquals(7L to 3L, BrowserCompanion.byteRange("bytes=-3", 10))
        assertEquals(0L to 10L, BrowserCompanion.byteRange("bytes=-30", 10))
        assertEquals(8L to 2L, BrowserCompanion.byteRange("bytes=8-100", 10))
        assertEquals(0L to 0L, BrowserCompanion.byteRange(null, 0))
        for (bad in listOf("bytes=10-", "bytes=3-2", "bytes=-0", "bytes=-", "bytes=0-1,3-4", "bytes=999999999999999999999-", "items=0-1")) {
            assertNull(bad, BrowserCompanion.byteRange(bad, 10))
        }
        assertNull(BrowserCompanion.byteRange("bytes=0-", 0))
    }

    @Test fun p2pCredentialsAndWifiQrEscaping() {
        assertEquals(android.net.wifi.p2p.WifiP2pConfig.GROUP_OWNER_BAND_5GHZ, WifiDirectCompanion.operatingBand(true))
        assertEquals(android.net.wifi.p2p.WifiP2pConfig.GROUP_OWNER_BAND_AUTO, WifiDirectCompanion.operatingBand(false))
        val (ssid, password) = WifiDirectCompanion.credentials()
        assertTrue(ssid.matches(Regex("DIRECT-[0-9a-f]{2}-AirBridge")))
        assertTrue(password.length in 8..63)
        assertNotEquals(password, WifiDirectCompanion.credentials().second)
        assertEquals("WIFI:T:WPA;S:DIRECT-ab-AirBridge;P:ab\\;c\\:d\\\\e;;", WifiDirectCompanion.wifiQr("DIRECT-ab-AirBridge", "ab;c:d\\e"))
        assertEquals(listOf("folder", "photo.jpg"), safeSegments("../../folder/../photo.jpg"))
    }

    @Test fun acceptedUploadUsesStorageAndRejectsDuplicateAndWrongOwner() = runBlocking {
        val platform = FakePlatform(tmp.newFolder(), "Phone")
        val engine = Engine(platform)
        val web = BrowserCompanion(engine) { ByteArray(0) }
        val response = async(Dispatchers.IO) {
            web.handle(request("/send", web.secret, "POST", """{"files":[{"name":"../../photo.jpg","size":5,"type":"image/jpeg"}]}"""))
        }
        val transfer = withTimeout(10_000) {
            while (engine.transfers.value.isEmpty()) delay(10)
            engine.transfers.value.first()
        }
        assertEquals(Status.WAITING, transfer.status)
        engine.decide(transfer.id, true)
        val prepared = response.await()
        assertEquals(200, prepared.status)
        val data = Protocol.json.parseToJsonElement(String(prepared.body)).jsonObject
        val params = mapOf("transfer" to data.getValue("transfer").jsonPrimitive.content, "file" to data.getValue("files").jsonArray.first().jsonObject.getValue("id").jsonPrimitive.content)
        val wrong = request("/upload", web.secret, "POST", "hello", params + ("id" to "wrong"))
        // request helper puts its default id last; construct the wrong owner explicitly.
        assertEquals(403, web.handle(HttpRequest("POST", "/upload", wrong.query + ("id" to "wrong"), emptyMap(), wrong.body, "127.0.0.1", null) {}).status)
        assertEquals(200, web.handle(request("/upload", web.secret, "POST", "hello", params)).status)
        assertEquals("hello", File(platform.downloads, "photo.jpg").readText())
        assertEquals(Status.DONE, engine.transfer(transfer.id)?.status)
        assertEquals(403, web.handle(request("/upload", web.secret, "POST", "hello", params)).status)
        web.close()
    }

    @Test fun invalidMetadataAndStoppingPendingRequest() = runBlocking {
        val engine = Engine(FakePlatform(tmp.newFolder(), "Phone"))
        val web = BrowserCompanion(engine) { ByteArray(0) }
        for (body in listOf(
            """{"files":[]}""",
            """{"files":[{"name":"","size":1}]}""",
            """{"files":[{"name":"file","size":-1}]}""",
            """{"files":[{"name":"a","size":9223372036854775807},{"name":"b","size":1}]}""",
        )) assertEquals(400, web.handle(request("/send", web.secret, "POST", body)).status)
        assertTrue(engine.transfers.value.isEmpty())
        val pending = async(Dispatchers.IO) {
            web.handle(request("/send", web.secret, "POST", """{"files":[{"name":"file","size":1}]}"""))
        }
        withTimeout(10_000) { while (engine.transfers.value.isEmpty()) delay(10) }
        web.close()
        assertEquals(403, pending.await().status)
        assertEquals(Status.CANCELLED, engine.transfers.value.first().status)
    }

    @Test fun downloadsRequireAcceptanceAndStreamRanges() = runBlocking {
        val platform = FakePlatform(tmp.newFolder(), "Phone")
        val engine = Engine(platform)
        val web = BrowserCompanion(engine) { ByteArray(0) }
        val file = tmp.newFile("source.txt").apply { writeText("0123456789") }
        val id = web.offer(listOf(file.path))!!
        val fileId = engine.transfer(id)!!.files.first().id
        val path = "/download/$id/$fileId"
        val offers = web.handle(request("/poll", web.secret))
        assertTrue(String(offers.body).contains(id))
        assertEquals(409, web.handle(request(path, web.secret)).status)
        assertEquals(200, web.handle(request("/offer/$id/accept", web.secret, "POST")).status)
        val res = web.handle(request(path, web.secret, headers = mapOf("range" to "bytes=2-5")))
        assertEquals(206, res.status)
        assertEquals("bytes 2-5/10", res.headers["Content-Range"])
        assertEquals(4L, res.length)
        val output = ByteArrayOutputStream()
        res.stream!!.invoke(output)
        assertEquals("2345", output.toString())
        assertEquals(Status.TRANSFERRING, engine.transfer(id)!!.status)
        val whole = web.handle(request(path, web.secret))
        whole.stream!!.invoke(ByteArrayOutputStream())
        assertEquals(Status.DONE, engine.transfer(id)!!.status)
        assertEquals(416, web.handle(request(path, web.secret, headers = mapOf("range" to "bytes=10-"))).status)
        val cancelled = web.offer(listOf(file.path))!!
        engine.cancel(cancelled)
        val cancellation = web.handle(request("/poll", web.secret))
        assertTrue(String(cancellation.body).contains("cancel"))
        assertTrue(String(cancellation.body).contains(cancelled))
        web.close()
    }

    @Test fun httpResponseStreamsWithHeadersAndClosesOnStop() {
        val server = HttpServer(null) { HttpResponse(206, contentType = "application/octet-stream", headers = mapOf("Content-Range" to "bytes 2-5/10"), length = 4, stream = { it.write("2345".toByteArray()) }) }
        val port = server.start(0, "127.0.0.1")
        try {
            Socket("127.0.0.1", port).use { socket ->
                socket.soTimeout = 5000
                socket.getOutputStream().write("GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".toByteArray())
                val response = socket.getInputStream().readBytes().toString(Charsets.UTF_8)
                assertTrue(response, response.startsWith("HTTP/1.1 206"))
                assertTrue(response, response.contains("Content-Length: 4\r\n"))
                assertTrue(response, response.endsWith("\r\n\r\n2345"))
            }
        } finally { server.stop() }
    }
}
