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
    val pin: String = "",
    val trusted: Set<String> = emptySet(),
    /** Run the background service so the phone is visible and can receive. */
    val receiving: Boolean = true,
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
