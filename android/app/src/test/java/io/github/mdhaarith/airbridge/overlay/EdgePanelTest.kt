package io.github.mdhaarith.airbridge.overlay

import android.content.Context
import android.view.View
import androidx.lifecycle.findViewTreeLifecycleOwner
import androidx.savedstate.findViewTreeSavedStateRegistryOwner
import app.cash.paparazzi.HtmlReportWriter
import app.cash.paparazzi.Paparazzi
import io.github.mdhaarith.airbridge.AirBridgeApp
import io.github.mdhaarith.airbridge.core.Engine
import io.github.mdhaarith.airbridge.core.FakePlatform
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class EdgePanelTest {
    @get:Rule
    val paparazzi = Paparazzi(snapshotHandler = HtmlReportWriter())

    @get:Rule
    val tmp = TemporaryFolder()

    @Test
    fun handleRootOwnsTheWindowRecomposerLifecycle() {
        // Exercise the real private handle without starting a service or window.
        val app = AirBridgeApp()
        val engine = Engine(FakePlatform(tmp.newFolder(), "Pixel"))
        AirBridgeApp::class.java.getDeclaredField("engine\$delegate").apply {
            isAccessible = true
        }.set(app, lazy { engine })
        val instance = AirBridgeApp::class.java.getDeclaredField("instance").apply { isAccessible = true }
        val previous = instance.get(null)
        instance.set(null, app)
        try {
            val panel = EdgePanel(paparazzi.context)
            try {
                val handle = Class.forName("${EdgePanel::class.java.name}\$HandleRoot")
                    .getDeclaredConstructor(EdgePanel::class.java, Context::class.java, Boolean::class.javaPrimitiveType, Boolean::class.javaPrimitiveType)
                    .apply { isAccessible = true }
                    .newInstance(panel, paparazzi.context, true, true) as View
                // Owners on the nested ComposeView cannot be found from the root.
                assertNotNull(handle.findViewTreeLifecycleOwner())
                assertNotNull(handle.findViewTreeSavedStateRegistryOwner())
            } finally {
                panel.destroy()
            }
        } finally {
            instance.set(null, previous)
        }
    }
}
