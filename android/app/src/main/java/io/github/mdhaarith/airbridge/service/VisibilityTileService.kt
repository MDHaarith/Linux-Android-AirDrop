package io.github.mdhaarith.airbridge.service

import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import io.github.mdhaarith.airbridge.AirBridgeApp

/** Quick Settings tile: turn receiving on/off without opening the app. */
class VisibilityTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        render()
    }

    override fun onClick() {
        super.onClick()
        if (AirBridgeApp.engine.settings.value.receiving) AirBridgeService.stop(this) else AirBridgeService.start(this)
        render()
    }

    private fun render() {
        val tile = qsTile ?: return
        val on = AirBridgeApp.engine.settings.value.receiving
        tile.state = if (on) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = "AirBridge"
        tile.subtitle = if (on) "Visible" else "Hidden"
        tile.updateTile()
    }
}
