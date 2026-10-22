package io.github.mdhaarith.airbridge.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import java.io.File

@Serializable
data class AppSettings(
    val alias: String = "",
    val port: Int = Protocol.DEFAULT_PORT,
    val autoAcceptTrusted: Boolean = true,
    val copyReceivedText: Boolean = true,
    /** What your own (trusted) devices send opens right away, like AirDrop between your devices. */
    val openReceived: Boolean = true,
    val pin: String = "",
    val trusted: Set<String> = emptySet(),
    /** Run the background service so the phone is visible and can receive. */
    val receiving: Boolean = true,
    /** A handle on the screen edge that opens a send panel over any app. */
    val edgePanel: Boolean = false,
    /** "right" or "left". */
    val edgeSide: String = "right",
    /** Vertical position of the handle, as a fraction of the screen height. */
    val edgeY: Float = 0.38f,
    /** Device id, or DropTargets.ALL for an explicit broadcast. */
    val edgeRecipient: String? = null,
)

/** Settings persisted as JSON in the app's private storage. */
class SettingsStore(private val dir: File, private val defaultAlias: String) {
    private val file get() = File(dir, "settings.json")
    private val _state = MutableStateFlow(load())
    val state: StateFlow<AppSettings> = _state.asStateFlow()
    val value get() = _state.value

    private fun normalize(s: AppSettings) = s.copy(
        alias = s.alias.trim().take(64).ifEmpty { defaultAlias },
        pin = s.pin.filter { it.isLetterOrDigit() }.take(32),
    )

    private fun load(): AppSettings {
        val stored = runCatching { Protocol.json.decodeFromString(AppSettings.serializer(), file.readText()) }.getOrNull()
        return normalize(stored ?: AppSettings())
    }

    @Synchronized
    fun update(change: (AppSettings) -> AppSettings) {
        val next = normalize(change(_state.value))
        dir.mkdirs()
        val tmp = File(dir, "settings.json.tmp")
        tmp.writeText(Protocol.json.encodeToString(AppSettings.serializer(), next))
        tmp.renameTo(file)
        _state.value = next
    }
}
