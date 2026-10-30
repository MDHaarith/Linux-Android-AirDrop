package io.github.mdhaarith.airbridge.service

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UpdaterTest {
    @Test
    fun parsesOnlyPlainReleaseTags() {
        assertEquals(listOf(1, 2, 3), Updater.version("v1.2.3"))
        assertEquals(listOf(0, 1, 5), Updater.version("0.1.5"))
        assertNull(Updater.version("v1.2"))
        assertNull(Updater.version("v1.2.3-rc1"))
        assertNull(Updater.version("v1.-2.3"))
    }

    @Test
    fun comparesNumericallyNotAsText() {
        assertTrue(Updater.isNewer("v0.1.10", "0.1.9"))
        assertTrue(Updater.isNewer("v1.0.0", "0.9.9"))
        assertFalse(Updater.isNewer("v0.1.5", "0.1.5"))
        assertFalse(Updater.isNewer("v0.1.4", "0.1.5"))
        assertFalse(Updater.isNewer("nightly", "0.1.5"))
    }
}
