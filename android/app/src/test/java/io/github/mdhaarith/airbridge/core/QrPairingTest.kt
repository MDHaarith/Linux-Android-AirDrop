package io.github.mdhaarith.airbridge.core

import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class QrPairingTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val fp = "AB".repeat(32)
    private val token = "a1".repeat(24)

    @Test
    fun codesSurviveTheRoundTrip() {
        val code = PairCode(fp, "192.168.49.1", 53317, token, "DIRECT-ab-AirBridge", "p&ss=w+rd 1",
            listOf("100.64.0.1", "10.2.3.4", "172.31.255.255", "192.168.1.20"))
        assertEquals(code, PairCode.parse(code.uri()))
        val plain = PairCode(fp, "192.168.1.20", 53317, token)
        assertEquals(plain, PairCode.parse(plain.uri()))
    }

    @Test
    fun extraAddressesAreValidatedBeforeAnyConnection() {
        val uri = PairCode(fp, "192.168.1.20", 53317, token).uri()
        assertEquals(listOf("100.127.255.255", "10.0.0.1"), PairCode.parse("$uri&a=100.127.255.255%2C10.0.0.1")?.addresses)
        listOf("", "example.com", "8.8.8.8", "127.0.0.1", "100.128.0.0", "100.63.255.255", "::1", "10.0.0.999",
            "10.0.0.1,", "10.0.0.1,evil.example", "10.0.0.1,10.0.0.2,10.0.0.3,10.0.0.4,10.0.0.5",
            "evil.example&a=10.0.0.1",
        ).forEach { assertNull(it, PairCode.parse("$uri&a=$it")) }
        assertEquals("8.8.8.8", PairCode.parse(PairCode(fp, "8.8.8.8", 53317, token).uri())?.ip)
    }

    @Test
    fun pairBodyIsRememberedAndInvalidBodiesDoNotSpendTheKey() = runBlocking {
        val host = Engine(FakePlatform(tmp.newFolder("body-host"), "Host"))
        val phone = Engine(FakePlatform(tmp.newFolder("body-phone"), "Phone"))
        host.start(); phone.start()
        try {
            val token = host.newPairToken()
            suspend fun post(body: String) = phone.client.post("127.0.0.1", host.port, true, host.identity.fingerprint,
                PairCode.PATH, mapOf("token" to token), PeerClient.json(body))
            assertEquals(400, post("""{"alias":"Phone","port":53317,"addresses":["evil.example"]}""").status)
            assertFalse(host.isTrusted(phone.identity.fingerprint))
            assertEquals(200, post("""{"alias":"  Phone  ","port":54317,"addresses":["100.64.0.2","192.168.1.2"]}""").status)
            // Both engines are live, so discovery may already have refreshed the port and put a real address first.
            val known = host.settings.value.known.single()
            assertEquals(phone.identity.fingerprint, known.fingerprint)
            assertEquals("Phone", known.alias)
            assertTrue(known.addresses.containsAll(listOf("100.64.0.2", "192.168.1.2")))
            assertEquals(403, post("""{"alias":"Phone","port":54317,"addresses":[]}""").status)

            val legacyToken = host.newPairToken()
            assertEquals(200, phone.client.post("127.0.0.1", host.port, true, host.identity.fingerprint,
                PairCode.PATH, mapOf("token" to legacyToken)).status)
        } finally { host.close(); phone.close() }
    }

    @Test
    fun anExtraAddressCanPairWhenTheFirstAddressHasTheWrongCertificate() = runBlocking {
        val ip = ownAddresses().firstOrNull()
        assumeTrue("requires a private network address", ip != null)
        val phone = Engine(FakePlatform(tmp.newFolder("fallback-phone"), "Phone"))
        val host = Identity.loadOrCreate(tmp.newFolder("fallback-host"))
        val wrong = Identity.loadOrCreate(tmp.newFolder("fallback-wrong"))
        var wrongRequests = 0
        var request: PairRequest? = null
        val wrongServer = HttpServer(wrong.serverContext) { wrongRequests++; HttpResponse.json(200, "{}") }
        val realServer = HttpServer(host.serverContext) { req ->
            if (req.path == PairCode.PATH) {
                request = PairRequest.parse(String(req.readBody()))
                HttpResponse.json(200, "{}")
            } else HttpResponse.json(200, Protocol.json.encodeToString(InfoDto.serializer(),
                InfoDto("Host", Protocol.VERSION, "JVM", "desktop", host.fingerprint, false)))
        }
        try {
            val port = wrongServer.start(55417, "127.0.0.1")
            assertEquals(port, realServer.start(port, ip))
            phone.start()
            assertTrue(phone.redeemPair(PairCode(host.fingerprint, "127.0.0.1", port, token,
                addresses = listOf(ip!!, "100.64.0.99"))))
            assertEquals(0, wrongRequests)
            assertEquals(PairRequest("Phone", phone.port, ownAddresses()), request)
            assertEquals(KnownDevice(host.fingerprint, "Host", port, listOf(ip, "100.64.0.99")), phone.settings.value.known.single())
        } finally { phone.close(); wrongServer.stop(); realServer.stop() }
    }

    @Test
    fun foreignAndHostileCodesAreRejected() {
        fun uri(f: String = fp, i: String = "192.168.1.20", p: String = "53317", t: String = token, extra: String = "") =
            "airbridge://pair?f=$f&i=$i&p=$p&t=$t$extra"
        assertTrue(PairCode.parse(uri()) != null)
        listOf(
            "https://example.com", "WIFI:T:WPA;S:x;P:y;;", "airbridge://pair?", "airbridge://pair?f",
            uri(f = "abc"), uri(t = "short"), uri(t = "$token%0A"), uri(p = "0"), uri(p = "70000"), uri(p = "x"),
            uri(i = "evil.example.com"), uri(i = "127.0.0.1"), uri(i = "0.0.0.0"), uri(i = "224.0.0.167"), uri(i = "999.1.1.1"),
            uri(extra = "&s=DIRECT-ab-x"), uri(extra = "&w=password1"),
            uri(extra = "&s=HomeWifi&w=password1"), uri(extra = "&s=DIRECT-ab-x&w=short"),
        ).forEach { assertNull(it, PairCode.parse(it)) }
    }

    @Test
    fun qrDecodesFromACameraFrame() {
        val text = PairCode(fp, "192.168.49.1", 53317, token, "DIRECT-ab-AirBridge", "a".repeat(24)).uri()
        val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 600, 600)
        val stride = 640 // Camera rows are often wider than the picture.
        val frame = ByteArray(stride * 600) { i ->
            val x = i % stride; val y = i / stride
            if (x < 600 && matrix[x, y]) 0 else -1
        }
        assertEquals(text, decodeQr(frame, stride, 600, 600))
        assertNull(decodeQr(ByteArray(stride * 600), stride, 600, 600))
    }

    @Test
    fun aQrKeyPairsOneDeviceBothWaysThenStopsWorking() = runBlocking {
        val host = Engine(FakePlatform(tmp.newFolder("host"), "Host"))
        val phone = Engine(FakePlatform(tmp.newFolder("phone"), "Phone"))
        val other = Engine(FakePlatform(tmp.newFolder("other"), "Other"))
        host.start(); phone.start(); other.start()
        try {
            val code = PairCode(host.identity.fingerprint, "127.0.0.1", host.port, host.newPairToken())
            assertFalse("a wrong key pairs nothing", phone.redeemPair(code.copy(token = "0".repeat(48))))
            assertFalse(host.isTrusted(phone.identity.fingerprint))

            assertTrue(phone.redeemPair(code))
            assertTrue(host.isTrusted(phone.identity.fingerprint))
            assertTrue(phone.isTrusted(host.identity.fingerprint))

            assertFalse("the key works once", other.redeemPair(code))
            assertFalse(host.isTrusted(other.identity.fingerprint))
        } finally {
            host.stop(); phone.stop(); other.stop()
        }
    }

    @Test
    fun aCodeForAnotherCertificateIsRefusedBeforeTheKeyIsSent() = runBlocking {
        val host = Engine(FakePlatform(tmp.newFolder("host"), "Host"))
        val phone = Engine(FakePlatform(tmp.newFolder("phone"), "Phone"))
        host.start(); phone.start()
        try {
            val token = host.newPairToken()
            val forged = PairCode("0".repeat(64), "127.0.0.1", host.port, token)
            assertTrue(runCatching { phone.redeemPair(forged) }.exceptionOrNull() is PinMismatchException)
            assertFalse(host.isTrusted(phone.identity.fingerprint))
            assertFalse(phone.isTrusted("0".repeat(64)))
            // The real host never saw the key, so it still works.
            assertTrue(phone.redeemPair(PairCode(host.identity.fingerprint, "127.0.0.1", host.port, token)))
        } finally {
            host.stop(); phone.stop()
        }
    }
}
