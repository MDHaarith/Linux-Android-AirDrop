package io.github.mdhaarith.airbridge

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import io.github.mdhaarith.airbridge.service.AirBridgeService
import io.github.mdhaarith.airbridge.ui.AirBridgeRoot
import io.github.mdhaarith.airbridge.ui.AirBridgeTheme
import io.github.mdhaarith.airbridge.ui.AppViewModel

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        if (AirBridgeApp.engine.settings.value.receiving) AirBridgeService.start(this)
        if (savedInstanceState == null) handleIntent(intent)
        setContent {
            AirBridgeTheme { AirBridgeRoot(vm) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** "Share → AirBridge" from any app, or a tap on an incoming-request notification. */
    private fun handleIntent(intent: Intent?) {
        intent ?: return
        intent.getStringExtra(EXTRA_TRANSFER)?.let { vm.showIncoming(it) }
        when (intent.action) {
            Intent.ACTION_SEND -> {
                val uri = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                if (uri != null) vm.stage(listOf(uri))
                else intent.getStringExtra(Intent.EXTRA_TEXT)?.let(vm::stageText)
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                val uris = IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
                if (uris.isNotEmpty()) vm.stage(uris)
            }
        }
    }

    companion object {
        const val EXTRA_TRANSFER = "transfer"
    }
}
