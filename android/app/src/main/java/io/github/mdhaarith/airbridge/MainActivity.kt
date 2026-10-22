package io.github.mdhaarith.airbridge

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.core.content.ContextCompat
import io.github.mdhaarith.airbridge.service.AirBridgeService
import io.github.mdhaarith.airbridge.ui.AirBridgeRoot
import io.github.mdhaarith.airbridge.ui.AirBridgeTheme
import io.github.mdhaarith.airbridge.ui.AppViewModel

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }
    private val photoAccess = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        // Asked for by the edge panel: back to the app the person was in.
        if (granted.values.any { it }) moveTaskToBack(true)
    }

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

    override fun onResume() {
        super.onResume()
        // Coming back from "Display over other apps" in system settings.
        if (AirBridgeApp.engine.settings.value.edgePanel) AirBridgeService.refreshEdgePanel(this)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    /** A tap on an incoming-request notification. ("Share → AirBridge" opens ShareSheetActivity.) */
    private fun handleIntent(intent: Intent?) {
        intent?.getStringExtra(EXTRA_TRANSFER)?.let { vm.showIncoming(it) }
        if (intent?.action == ACTION_ASK_PHOTOS && !canReadPhotos(this)) photoAccess.launch(photoPermissions)
    }

    companion object {
        const val EXTRA_TRANSFER = "transfer"
        /** From the edge panel: ask for photo access so it can offer the latest photo. */
        const val ACTION_ASK_PHOTOS = "io.github.mdhaarith.airbridge.action.ASK_PHOTOS"
    }
}
