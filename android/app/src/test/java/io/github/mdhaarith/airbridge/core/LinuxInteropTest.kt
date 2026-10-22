package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import kotlin.random.Random

/**
 * End-to-end test of the Android networking core against the real Linux app.
 *
 * Runs only when AIRBRIDGE_LINUX_BIN points to the `airbridge` binary:
 *   AIRBRIDGE_LINUX_BIN="$PWD/../linux/target/debug/airbridge" ./gradlew testDebugUnitTest
 *
 * It exercises multicast discovery, the mutual-TLS register handshake in both
 * directions, files and text both ways, trust-based auto-accept and declining.
 */
class LinuxInteropTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val bin = System.getenv("AIRBRIDGE_LINUX_BIN").orEmpty()
    private lateinit var linuxHome: File
    private lateinit var linuxLog: File
    private var linux: Process? = null
    private lateinit var platform: FakePlatform
    private lateinit var engine: Engine

    @Before
    fun setUp() {
        assumeTrue("set AIRBRIDGE_LINUX_BIN to run the interop test", bin.isNotEmpty() && File(bin).canExecute())
        platform = FakePlatform(tmp.newFolder("phone"), "Kotlin Phone")
        engine = Engine(platform)

        // The Linux side trusts the phone, so it accepts without a prompt.
        linuxHome = tmp.newFolder("linux")
        File(linuxHome, "config").mkdirs()
        File(linuxHome, "config/settings.json").writeText(
            """{"alias":"Linux Test","port":54317,"webPort":54318,"autoAcceptTrusted":true,"trusted":["${engine.identity.fingerprint}"],"notifications":false}""",
        )
        linuxLog = File(linuxHome, "log.txt")
        linux = ProcessBuilder(bin, "daemon")
            .apply { environment()["AIRBRIDGE_HOME"] = linuxHome.path; environment()["NO_COLOR"] = "1"; environment()["AIRBRIDGE_LOG"] = "debug,zbus=warn,hyper=info,rustls=warn" }
            .redirectErrorStream(true)
            .redirectOutput(linuxLog)
            .start()
    }

    @After
    fun tearDown() {
        if (::engine.isInitialized) engine.close()
        linux?.destroy()
        linux?.waitFor()
        // Keep the Linux side's log for debugging failures.
        if (::linuxLog.isInitialized) linuxLog.copyTo(File("build/linux-interop.log"), overwrite = true)
    }

    private fun linuxFingerprint(): String {
        repeat(100) {
            Regex("Fingerprint ([0-9A-F]{64})").find(linuxLog.readText())?.let { return it.groupValues[1] }
            Thread.sleep(100)
        }
        error("Linux app did not start:\n${linuxLog.readText()}")
    }

    private fun cli(vararg args: String): Pair<Int, String> {
        val process = ProcessBuilder(bin, *args)
            .apply { environment()["AIRBRIDGE_HOME"] = linuxHome.path; environment()["NO_COLOR"] = "1" }
            .redirectErrorStream(true)
            .start()
        val output = process.inputStream.bufferedReader().readText()
        return process.waitFor() to output
    }

    /** Width and height from a JPEG's start-of-frame marker. */
    private fun jpegSize(jpeg: ByteArray): Pair<Int, Int> {
        var i = 2
        while (i + 9 < jpeg.size) {
            val marker = jpeg[i + 1].toInt() and 0xff
            val length = ((jpeg[i + 2].toInt() and 0xff) shl 8) or (jpeg[i + 3].toInt() and 0xff)
            if (marker in 0xc0..0xc2) {
                fun u16(at: Int) = ((jpeg[at].toInt() and 0xff) shl 8) or (jpeg[at + 1].toInt() and 0xff)
                return u16(i + 7) to u16(i + 5)
            }
            i += 2 + length
        }
        error("not a JPEG")
    }

    private suspend fun awaitTransfer(predicate: (Transfer) -> Boolean): Transfer =
        engine.transfers.first { list -> list.any(predicate) }.first(predicate)

    @Test
    fun phoneAndLinuxShareBothWays() = runBlocking {
        val linuxFp = linuxFingerprint()
        engine.settings.update { it.copy(port = 54417, trusted = setOf(linuxFp)) }
        engine.start()

        // 1. Discovery: multicast announcement + mutual-TLS register. Some CI
        //    networks filter multicast; then fall back to a direct register,
        //    which is what "Scan network" does.
        val viaMulticast = withTimeoutOrNull(12_000) { engine.peers.first { list -> list.any { it.id == linuxFp } } } != null
        if (!viaMulticast) {
            println("multicast unavailable here; registering directly")
            engine.discovery!!.registerWith("127.0.0.1", 54317, true, linuxFp, 4_000)
        }
        val laptop = withTimeout(10_000) { engine.peers.first { list -> list.any { it.id == linuxFp } } }.first { it.id == linuxFp }
        assertEquals("Linux Test", laptop.alias)

        // 2. Phone -> Linux: a file (Linux auto-accepts trusted devices).
        val payload = Random.nextBytes(3_000_000)
        val photo = tmp.newFile("holiday.jpg").apply { writeBytes(payload) }
        val sendId = engine.sendFiles(laptop, listOf(photo.path))!!
        val sent = withTimeout(30_000) { awaitTransfer { it.id == sendId && it.status.isFinal } }
        assertEquals("${sent.error} peer=$laptop peers=${engine.peers.value}", Status.DONE, sent.status)
        assertArrayEquals(payload, File(linuxHome, "downloads/holiday.jpg").readBytes())

        // 3. Phone -> Linux: text message (answered with 204, no upload).
        val textId = engine.sendText(laptop, "Hello from Kotlin ✨")
        assertEquals(Status.DONE, withTimeout(15_000) { awaitTransfer { it.id == textId && it.status.isFinal } }.status)

        // 4. Linux -> phone: a folder with nested files, auto-accepted (trusted).
        val folder = tmp.newFolder("Album")
        File(folder, "a.txt").writeText("first")
        File(folder, "sub").mkdirs()
        File(folder, "sub/b.bin").writeBytes(payload)
        val (code, out) = cli("send", folder.path, "--to", "Kotlin Phone")
        assertEquals(out, 0, code)
        assertTrue(engine.transfers.value.last { it.direction == Direction.IN && it.message == null }.autoAccepted)
        assertEquals("first", File(platform.downloads, "Album/a.txt").readText())
        assertArrayEquals(payload, File(platform.downloads, "Album/sub/b.bin").readBytes())

        // 5. Linux -> phone: text lands in the clipboard because the laptop is trusted.
        val (textCode, textOut) = cli("text", "Clipboard from Linux", "--to", "Kotlin Phone")
        assertEquals(textOut, 0, textCode)
        assertEquals("Clipboard from Linux", platform.clipboard.last())

        // 6. Untrusted: the phone is asked, with a preview of the photo like
        //    AirDrop, and declining is reported to the sender.
        engine.settings.update { it.copy(trusted = emptySet()) }
        val picture = tmp.newFile("sunset.jpg").apply { writeBytes(LinuxInteropTest::class.java.getResource("/sunset.jpg")!!.readBytes()) }
        var asked: Transfer? = null
        val asking = Thread {
            runBlocking {
                val incoming = withTimeout(20_000) { awaitTransfer { it.direction == Direction.IN && it.status == Status.WAITING } }
                asked = incoming
                engine.decide(incoming.id, accept = false)
            }
        }.apply { start() }
        val (declinedCode, declinedOut) = cli("send", picture.path, "--to", "Kotlin Phone")
        asking.join()
        val preview = Previews.decode(asked!!.files.single().preview)
        assertNotNull("the request carries a preview", preview)
        assertEquals(360 to 240, jpegSize(preview!!))
        assertEquals("a photo", sharePhrase(asked!!.files))
        assertFalse(asked!!.autoAccepted)
        assertNotEquals(0, declinedCode)
        assert(declinedOut.contains("Declined", ignoreCase = true)) { declinedOut }
    }
}
