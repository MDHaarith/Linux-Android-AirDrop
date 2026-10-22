package io.github.mdhaarith.airbridge.share

import io.github.mdhaarith.airbridge.core.Peer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ShareLogicTest {
    private fun peer(id: String, alias: String = id) = Peer(id, alias, null, "desktop", "10.0.0.2", 53317, true, 0)

    @Test
    fun filesWinOverCaptions() {
        val request = ShareRequest.from(ShareRequest.SEND, listOf("content://a", "content://a"), "Look at this")
        assertEquals(ShareRequest(listOf("content://a"), null), request)
    }

    @Test
    fun linksAndTextAreSentAsText() {
        assertEquals(ShareRequest(emptyList(), "https://example.com/x"), ShareRequest.from(ShareRequest.SEND, emptyList(), "  https://example.com/x \n"))
        assertNull(ShareRequest.from(ShareRequest.SEND, emptyList(), "   "))
        assertNull(ShareRequest.from("android.intent.action.VIEW", listOf("content://a"), null))
        // Our own private files can never be pushed into a share.
        assertNull(ShareRequest.from(ShareRequest.SEND, listOf("file:///data/user/0/io.github.mdhaarith.airbridge/files/identity/identity.p12"), null))
        assertNull(ShareRequest.from(ShareRequest.SEND, listOf("content://io.github.mdhaarith.airbridge.files/identity"), null))
        assertEquals(
            ShareRequest(listOf("content://media/external/images/media/12"), null),
            ShareRequest.from(ShareRequest.SEND, listOf("content://media/external/images/media/12", "file:///sdcard/x.jpg"), null),
        )
    }

    @Test
    fun shortcutIdsRoundTrip() {
        assertEquals("ABC", ShortcutIds.peerId(ShortcutIds.of("ABC")))
        assertNull(ShortcutIds.peerId("something-else"))
        assertNull(ShortcutIds.peerId(null))
        assertNull(ShortcutIds.peerId("device:"))
    }

    @Test
    fun plannerRanksTrustedFirstAndOnlyReportsChanges() {
        val planner = ShortcutPlanner(max = 2, keepMillis = 1_000)
        val plan = planner.update(listOf(peer("a"), peer("b"), peer("c")), trusted = setOf("c"), now = 0)!!
        assertEquals(listOf("c", "a"), plan.publish.map { it.id })
        assertEquals(emptyList<String>(), plan.remove)

        // Same devices again: nothing to do (the peer list updates constantly).
        assertNull(planner.update(listOf(peer("a"), peer("b"), peer("c")), setOf("c"), now = 100))

        // A rename is republished.
        val renamed = planner.update(listOf(peer("a", "Laptop"), peer("c")), setOf("c"), now = 200)!!
        assertEquals("Laptop", renamed.publish.first { it.id == "a" }.alias)

        // Devices that left are kept for a while, then removed.
        assertNull(planner.update(listOf(peer("c")), setOf("c"), now = 900))
        val later = planner.update(listOf(peer("c")), setOf("c"), now = 2_500)!!
        assertEquals(listOf("c"), later.publish.map { it.id })
        assertEquals(listOf("a"), later.remove)
    }
}
