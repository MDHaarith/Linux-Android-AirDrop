package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class KnownDevicesTest {
    @get:Rule val tmp = TemporaryFolder()
    private val fp = "AB".repeat(32)

    @Test
    fun onlyLiteralPrivateIpv4AddressesAreAllowed() {
        listOf("10.0.0.0", "10.255.255.255", "172.16.0.0", "172.31.255.255", "192.168.0.0", "192.168.255.255",
            "100.64.0.0", "100.127.255.255").forEach { assertTrue(it, isPrivateIpv4(it)) }
        listOf("", "localhost", "10.0.0.1.example", "127.0.0.1", "0.0.0.0", "169.254.1.2", "8.8.8.8", "224.0.0.167",
            "172.15.255.255", "172.32.0.0", "100.63.255.255", "100.128.0.0", "192.169.0.0", "10.256.0.0", "10.0.0",
            "10.0.0.1.2", "10.00.0.1", "10.0.0.-1", "10.0.0.+1", " 10.0.0.1", "10.0.0.1\n", "::1", "::ffff:10.0.0.1")
            .forEach { assertFalse(it, isPrivateIpv4(it)) }
        val own = ownAddresses()
        assertTrue(own.size <= 4)
        assertEquals(own.distinct(), own)
        assertTrue(own.all(::isPrivateIpv4))
    }

    @Test
    fun pairBodiesAreBoundedAndSanitized() {
        assertNull(PairRequest.parse(""))
        assertNull(PairRequest.parse(" \n"))
        val addresses = (1..8).map { "100.64.0.$it" }
        val body = PairRequest("  \u200B" + "x".repeat(140) + "  ", 1024, addresses)
        val parsed = PairRequest.parse(Protocol.json.encodeToString(PairRequest.serializer(), body))!!
        assertEquals("x".repeat(128), parsed.alias)
        assertEquals(addresses, parsed.addresses)
        listOf(body.copy(port = 1023), body.copy(port = 65536), body.copy(alias = " \u200B"),
            body.copy(addresses = addresses + "10.0.0.1"), body.copy(addresses = listOf("example.com")),
            body.copy(addresses = listOf("8.8.8.8")), body.copy(addresses = listOf("127.0.0.1")))
            .forEach { assertTrue(it.toString(), runCatching { PairRequest.parse(Protocol.json.encodeToString(PairRequest.serializer(), it)) }.isFailure) }
        listOf("{}", "null", "[]", "garbage", """{"alias":"Phone","port":53317,"addresses":null}""")
            .forEach { assertTrue(it, runCatching { PairRequest.parse(it) }.isFailure) }
    }

    @Test
    fun rememberingRequiresTrustAndKeepsSixRecentPrivateAddresses() {
        val device = KnownDevice(fp, " Phone ", 53317, (1..8).map { "100.64.0.$it" } + "example.com")
        assertEquals(emptyList<KnownDevice>(), AppSettings().remember(device).known)
        val settings = AppSettings(trusted = setOf(fp)).remember(device)
        assertEquals((1..6).map { "100.64.0.$it" }, settings.known.single().addresses)
        assertEquals("Phone", settings.known.single().alias)
        val updated = settings.remember(device.copy(alias = "Renamed", port = 54317, addresses = listOf("192.168.1.2", "100.64.0.3", "8.8.8.8")))
        assertEquals(KnownDevice(fp, "Renamed", 54317, listOf("192.168.1.2", "100.64.0.3", "100.64.0.1", "100.64.0.2", "100.64.0.4", "100.64.0.5")),
            updated.known.single())
        assertEquals(updated, updated.remember(updated.known.single()))
        assertEquals(updated, updated.remember(device.copy(port = 80)))
    }

    @Test
    fun trustedTlsUpdatesRefreshTheBookAndUntrustingRemovesIt() {
        val engine = Engine(FakePlatform(tmp.newFolder(), "Phone"))
        try {
            val info = DeviceInfo("Laptop", fingerprint = fp)
            engine.upsertPeer(info, "100.64.0.1", verified = true)
            assertTrue(engine.settings.value.known.isEmpty())
            engine.setTrusted(fp, true)
            assertEquals(listOf("100.64.0.1"), engine.settings.value.known.single().addresses)
            engine.upsertPeer(info.copy(alias = "Renamed", port = 54317), "192.168.1.2", verified = true)
            assertEquals(KnownDevice(fp, "Renamed", 54317, listOf("192.168.1.2", "100.64.0.1")), engine.settings.value.known.single())
            engine.upsertPeer(info, "10.0.0.1", verified = false)
            assertEquals("192.168.1.2", engine.settings.value.known.single().addresses.first())
            engine.setTrusted(fp, false)
            assertTrue(engine.settings.value.known.isEmpty())
            engine.upsertPeer(info, "100.64.0.1", verified = true)
            assertTrue(engine.settings.value.known.isEmpty())
        } finally { engine.close() }
    }

    @Test
    fun oldSettingsLoadAndUnchangedSettingsAreNotWritten() {
        val dir = tmp.newFolder()
        val file = File(dir, "settings.json")
        file.writeText("""{"alias":"Phone","trusted":[]}""")
        val store = SettingsStore(dir, "Phone")
        assertTrue(store.value.known.isEmpty())
        store.update { it.copy(trusted = setOf(fp)).remember(KnownDevice(fp, "Laptop", 53317, listOf("100.64.0.1"))) }
        assertEquals(store.value, SettingsStore(dir, "Phone").value)
        assertTrue(file.setLastModified(10_000))
        store.update { it.remember(it.known.single()) }
        assertEquals(10_000L, file.lastModified())
    }

    @Test
    fun rememberedDevicesReconnectWithTheirCertificatePinned() = runBlocking {
        val ip = ownAddresses().firstOrNull()
        assumeTrue("requires a private network address", ip != null)
        val engine = Engine(FakePlatform(tmp.newFolder(), "Phone"))
        val host = Identity.loadOrCreate(tmp.newFolder())
        val server = HttpServer(host.serverContext) {
            HttpResponse.json(200, Protocol.json.encodeToString(InfoDto.serializer(), InfoDto("Laptop", Protocol.VERSION, "JVM", "desktop", host.fingerprint, false)))
        }
        val port = server.start(55317)
        val discovery = Discovery(FakePlatform(tmp.newFolder(), "Unused"), engine)
        try {
            val device = KnownDevice(host.fingerprint, "Old alias", port, listOf(ip!!))
            engine.settings.update { it.copy(trusted = setOf(host.fingerprint), known = listOf(device)) }
            assertEquals(1, discovery.reconnectKnown())
            assertEquals("Laptop", engine.peers.value.single().alias)
            assertTrue(engine.peers.value.single().verified)
            assertEquals(0, discovery.reconnectKnown())
            engine.peerFailed(host.fingerprint); engine.peerFailed(host.fingerprint)
            engine.settings.update { it.copy(trusted = emptySet()) }
            assertEquals(0, discovery.reconnectKnown())
            engine.settings.update { it.copy(trusted = setOf(fp), known = listOf(device.copy(fingerprint = fp))) }
            assertEquals(0, discovery.reconnectKnown())
            assertTrue(engine.peers.value.isEmpty())
        } finally { server.stop(); engine.close() }
    }
}
