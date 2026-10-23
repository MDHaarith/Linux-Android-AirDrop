package io.github.mdhaarith.airbridge.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class DropTabLayoutTest {
    private val targets = listOf(
        DropTarget("all", "Send to all", null, everyone = true),
        DropTarget("a", "Laptop", null),
        DropTarget("b", "Phone", null),
    )

    @Test
    fun hubIsTheDefaultDeviceThenTheRestInOrder() {
        assertEquals(listOf("b", "all", "a"), DropTabLayout.arrange(targets, "b").map { it.id })
        assertEquals(listOf("a", "all", "b"), DropTabLayout.arrange(targets, null).map { it.id })
        assertEquals(emptyList<DropTarget>(), DropTabLayout.arrange(emptyList(), null))
    }

    @Test
    fun eachSpotHitsItselfOnBothSidesInEveryStyle() {
        for (style in DropStyle.entries) for (right in listOf(true, false)) {
            val spots = DropTabLayout.spots(style, 5, right)
            spots.centers.forEachIndexed { i, c ->
                assertEquals("$style right=$right", i, DropTabLayout.hit(style, 5, right, c.x, c.y))
            }
            // The hub sits against the chosen edge.
            val hub = spots.centers[0]
            assertEquals(right, hub.x > spots.width / 2)
        }
    }
}
