package io.github.mdhaarith.airbridge.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PreviewsTest {
    @Test
    fun onlySmallRasterPreviewsAreAccepted() {
        val url = Previews.dataUrl(byteArrayOf(1, 2, 3))
        assertEquals("data:image/jpeg;base64,AQID", url)
        assertArrayEquals(byteArrayOf(1, 2, 3), Previews.decode(url))
        assertNull(Previews.accept("data:image/svg+xml;base64,AAAA"))
        assertNull(Previews.accept("Hello, this is a text message"))
        assertNull(Previews.accept("data:image/png;base64," + "A".repeat(Previews.MAX_DATA_URL)))
        assertNull(Previews.decode("data:image/jpeg;base64,not base64!"))
    }

    @Test
    fun sharesAreWordedLikeAirDrop() {
        fun f(name: String, mime: String) = TFile(name, name, 1, mime)
        assertEquals("a photo", sharePhrase(listOf(f("a.jpg", "image/jpeg"))))
        assertEquals("2 photos", sharePhrase(listOf(f("a.jpg", "image/jpeg"), f("b.png", "image/png"))))
        assertEquals("a video", sharePhrase(listOf(f("clip.mp4", "video/mp4"))))
        assertEquals("“Report.pdf”", sharePhrase(listOf(f("Docs/Report.pdf", "application/pdf"))))
        assertEquals("2 items", sharePhrase(listOf(f("a.jpg", "image/jpeg"), f("b.pdf", "application/pdf"))))
    }
}
