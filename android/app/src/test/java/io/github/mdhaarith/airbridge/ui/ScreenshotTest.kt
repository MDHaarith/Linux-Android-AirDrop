package io.github.mdhaarith.airbridge.ui

import androidx.activity.compose.LocalActivityResultRegistryOwner
import androidx.activity.result.ActivityResultRegistry
import androidx.activity.result.ActivityResultRegistryOwner
import androidx.activity.result.contract.ActivityResultContract
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.core.app.ActivityOptionsCompat
import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import com.android.resources.NightMode
import io.github.mdhaarith.airbridge.core.DeviceInfo
import io.github.mdhaarith.airbridge.core.Direction
import io.github.mdhaarith.airbridge.core.Engine
import io.github.mdhaarith.airbridge.core.FakePlatform
import io.github.mdhaarith.airbridge.core.FileStatus
import io.github.mdhaarith.airbridge.core.Status
import io.github.mdhaarith.airbridge.core.TFile
import io.github.mdhaarith.airbridge.core.Transfer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * Renders the Android screens to PNG on the JVM (no emulator needed):
 *   ./gradlew recordPaparazziDebug   → app/src/test/snapshots/images
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ScreenshotTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_6.copy(nightMode = NightMode.NIGHT, softButtons = false),
        theme = "android:Theme.Material.NoActionBar",
        showSystemUi = false,
        maxPercentDifference = 0.5,
    )

    @get:Rule
    val tmp = TemporaryFolder()

    @Before
    fun setUp() = Dispatchers.setMain(UnconfinedTestDispatcher())

    @After
    fun tearDown() = Dispatchers.resetMain()

    private val laptop = "3C8AC97385EA40FCC3FDEF6299594DFAADB8F352BC451D4EFBBA556E6DB9BE81"
    private val imac = "9BC62DA2B2C545608C7B72DA1EDF2509D7869C543859EBBB7038D3AB574F1B6D"
    private val tablet = "C4E5E2E18372A6C80400BE8C4955AF49055CB5AF9284EA1817706F381492930B"

    private fun engine(): Engine {
        val engine = Engine(FakePlatform(tmp.newFolder(), "Pixel 8"))
        engine.upsertPeer(DeviceInfo("Mohamed's ThinkPad", deviceModel = "Fedora", deviceType = "desktop", fingerprint = laptop), "192.168.1.20")
        engine.upsertPeer(DeviceInfo("Office iMac", deviceModel = "macOS", deviceType = "desktop", fingerprint = imac), "192.168.1.31")
        engine.upsertPeer(DeviceInfo("Galaxy Tab", deviceModel = "Samsung SM-X710", deviceType = "mobile", fingerprint = tablet), "192.168.1.44")
        engine.settings.update { it.copy(trusted = setOf(laptop)) }
        val now = System.currentTimeMillis()
        engine.seedForPreview(
            listOf(
                Transfer(
                    "t1", Direction.IN, imac, "Office iMac", "desktop", "macOS", Status.DONE,
                    listOf(TFile("f1", "Quarterly report.pdf", 2_400_000, "application/pdf", 2_400_000, FileStatus.DONE, "x")),
                    created = now - 3_600_000, started = now - 3_600_000, finished = now - 3_590_000,
                ),
                Transfer(
                    "t2", Direction.IN, laptop, "Mohamed's ThinkPad", "desktop", "Fedora", Status.DONE,
                    listOf(TFile("m", "m.txt", 40, "text/plain", 40, FileStatus.DONE)),
                    message = "https://github.com/MDHaarith/Linux-Android-AirDrop",
                    created = now - 600_000, started = now - 600_000, finished = now - 600_000,
                ),
                Transfer(
                    "t3", Direction.OUT, laptop, "Mohamed's ThinkPad", "desktop", "Fedora", Status.TRANSFERRING,
                    listOf(
                        TFile("a", "IMG_2041.jpg", 4_200_000, "image/jpeg", 4_200_000, FileStatus.DONE),
                        TFile("b", "IMG_2042.jpg", 3_900_000, "image/jpeg", 1_300_000, FileStatus.TRANSFERRING),
                        TFile("c", "Trip video.mp4", 48_000_000, "video/mp4"),
                    ),
                    created = now - 4_000, started = now - 3_000,
                ),
            ),
        )
        return engine
    }

    private val noResults = object : ActivityResultRegistryOwner {
        override val activityResultRegistry = object : ActivityResultRegistry() {
            override fun <I, O> onLaunch(requestCode: Int, contract: ActivityResultContract<I, O>, input: I, options: ActivityOptionsCompat?) = Unit
        }
    }

    private fun shoot(content: @Composable () -> Unit) = paparazzi.snapshot {
        CompositionLocalProvider(LocalActivityResultRegistryOwner provides noResults, LocalInspectionMode provides true) {
            AirBridgeTheme { content() }
        }
    }

    @Test
    fun home() = shoot { AirBridgeRoot(AppViewModel(engine())) }

    @Test
    fun pickADevice() {
        val vm = AppViewModel(engine())
        vm.stageText("Meeting notes: ship AirBridge 0.1 on Friday")
        shoot { AirBridgeRoot(vm) }
    }

    @Test
    fun activity() {
        val vm = AppViewModel(engine()).apply { screen = Screen.ACTIVITY }
        shoot { AirBridgeRoot(vm) }
    }

    @Test
    fun settings() {
        val vm = AppViewModel(engine()).apply { screen = Screen.SETTINGS }
        shoot { AirBridgeRoot(vm) }
    }

    @Test
    fun incomingRequest() = shoot {
        val air = LocalAir.current
        Box(Modifier.fillMaxSize().background(air.bg)) {
            Aurora(Modifier.fillMaxSize())
            Box(
                Modifier.align(Alignment.BottomCenter).clip(RoundedCornerShape(topStart = 28.dp(), topEnd = 28.dp())).background(air.solid).padding(top = 20.dp()),
            ) {
                IncomingContent(
                    Transfer(
                        "in", Direction.IN, laptop, "Mohamed's ThinkPad", "desktop", "Fedora", Status.WAITING,
                        listOf(
                            TFile("1", "Resume.pdf", 180_000, "application/pdf"),
                            TFile("2", "Screenshot 2026-10-21.png", 1_200_000, "image/png"),
                            TFile("3", "airbridge-0.1.0.apk", 5_900_000, "application/vnd.android.package-archive"),
                        ),
                    ),
                    onDecline = {}, onAccept = {},
                )
            }
        }
    }
}

private fun Int.dp() = androidx.compose.ui.unit.Dp(this.toFloat())
