package io.github.mdhaarith.airbridge.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.FileOpen
import androidx.compose.material.icons.rounded.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.unit.dp
import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import com.android.resources.NightMode
import io.github.mdhaarith.airbridge.core.Peer
import org.junit.Rule
import org.junit.Test

/** The share sheet and the edge panel, floating over another app. */
class ShareScreenshotTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_6.copy(nightMode = NightMode.NIGHT, softButtons = false),
        theme = "android:Theme.Material.NoActionBar",
        showSystemUi = false,
        maxPercentDifference = 0.5,
    )

    private val laptop = Peer("3C8AC97385EA40FCC3FDEF6299594DFAADB8F352BC451D4EFBBA556E6DB9BE81", "Mohamed's ThinkPad", "Fedora", "desktop", "192.168.1.20", 53317, true, 0)
    private val imac = Peer("9BC62DA2B2C545608C7B72DA1EDF2509D7869C543859EBBB7038D3AB574F1B6D", "Office iMac", "macOS", "desktop", "192.168.1.31", 53317, true, 0)
    private val tablet = Peer("C4E5E2E18372A6C80400BE8C4955AF49055CB5AF9284EA1817706F381492930B", "Galaxy Tab", "Samsung SM-X710", "mobile", "192.168.1.44", 53317, true, 0)
    private val peers = listOf(laptop, imac, tablet)

    /** A stand-in for a photo: a sunset gradient with a sun. */
    private fun photo(): ImageBitmap {
        val image = ImageBitmap(160, 160)
        val canvas = Canvas(image)
        val paint = Paint()
        paint.shader = androidx.compose.ui.graphics.LinearGradientShader(Offset(0f, 0f), Offset(0f, 160f), listOf(Color(0xFF3E68BA), Color(0xFFFAB278)))
        canvas.drawRect(0f, 0f, 160f, 160f, paint)
        canvas.drawCircle(Offset(105f, 70f), 26f, Paint().apply { color = Color(0xFFFFE08A) })
        canvas.drawRect(0f, 118f, 160f, 160f, Paint().apply { color = Color(0xFF34466E) })
        return image
    }

    /** Some app underneath: a browser-like page. */
    @Composable
    private fun FakeApp() {
        Column(Modifier.fillMaxSize().background(Color(0xFFF4F4F7)).padding(20.dp)) {
            Box(Modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(22.dp)).background(Color(0xFFE3E4EA)))
            Spacer(Modifier.height(20.dp))
            Box(Modifier.fillMaxWidth().height(220.dp).clip(RoundedCornerShape(18.dp)).background(Brush.verticalGradient(listOf(Color(0xFF3E68BA), Color(0xFFFAB278)))))
            repeat(6) {
                Spacer(Modifier.height(14.dp))
                Box(Modifier.fillMaxWidth(if (it % 2 == 0) 0.92f else 0.7f).height(14.dp).clip(RoundedCornerShape(7.dp)).background(Color(0xFFDADBE3)))
            }
        }
    }

    private fun shoot(content: @Composable () -> Unit) = paparazzi.snapshot {
        CompositionLocalProvider(LocalInspectionMode provides true) {
            AirBridgeTheme(dark = false) {
                Box(Modifier.fillMaxSize()) {
                    FakeApp()
                    content()
                }
            }
        }
    }

    private val preview get() = SharePreview("sunset-at-the-pier.jpg", "2.4 MB", "image/jpeg", thumbnail = photo())

    private fun sheet(ui: ShareSheetUi) = shoot {
        ShareSheetContent(ui, onPick = {}, onDismiss = {}, onRetry = {}, onScan = {})
    }

    @Test
    fun shareSheet() = sheet(ShareSheetUi(preview, peers, trusted = setOf(laptop.id)))

    @Test
    fun shareSheetSending() = sheet(ShareSheetUi(preview, peers, setOf(laptop.id), targetId = laptop.id, phase = SharePhase.SENDING, progress = 0.64f))

    @Test
    fun shareSheetSent() = sheet(ShareSheetUi(preview, peers, setOf(laptop.id), targetId = laptop.id, phase = SharePhase.DONE, progress = 1f))

    @Test
    fun shareSheetLink() = sheet(
        ShareSheetUi(
            SharePreview("github.com", "https://github.com/MDHaarith/Linux-Android-AirDrop", isLink = true),
            peers = emptyList(),
        ),
    )

    @Test
    fun shareSheetDeclined() = sheet(
        ShareSheetUi(preview, peers, targetId = imac.id, phase = SharePhase.FAILED, error = "Office iMac declined"),
    )

    private fun actions(selected: String) = listOf(
        EdgeAction("clipboard", "Clipboard", "Flight AF 1234 lands 18:40, terminal 2E", Icons.Rounded.ContentPaste),
        EdgeAction("photo", "Latest photo", "2 min ago", Icons.Rounded.Image, thumbnail = photo()),
        EdgeAction("files", "Choose files…", "Pick anything to send", Icons.Rounded.FileOpen),
    ).let { list -> list to selected }

    @Test
    fun edgePanel() {
        val (list, selected) = actions("photo")
        shoot { EdgePanelContent(EdgePanelUi(true, list, selected, peers, setOf(laptop.id)), onSelect = {}, onPick = {}, onClose = {}) }
    }

    @Test
    fun edgePanelSending() {
        val (list, selected) = actions("clipboard")
        shoot {
            EdgePanelContent(
                EdgePanelUi(true, list, selected, peers, setOf(laptop.id), targetId = laptop.id, phase = SharePhase.WAITING),
                onSelect = {}, onPick = {}, onClose = {},
            )
        }
    }
}
