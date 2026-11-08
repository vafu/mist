package io.github.vafu.mist

import io.github.vafu.mist.core.ClipboardContent
import io.github.vafu.mist.core.ManagedDevice
import kotlinx.coroutines.flow.StateFlow

/** Lifecycle-neutral controller consumed by the shared mobile UI. */
public interface MistController : AutoCloseable {
    public val state: StateFlow<MistControllerState>

    public suspend fun initiatePairing(): String
    public fun cancelPairing()
    public suspend fun acceptPairing(offer: String)
    public suspend fun selectDevice(endpointId: String, selected: Boolean)
    public suspend fun connectDevice(endpointId: String)
    public suspend fun disconnectDevice(endpointId: String)
    public suspend fun forgetDevice(endpointId: String)

    /** Reads the focused app clipboard and sends it to the selected devices. */
    public suspend fun sendClipboard(targets: List<String> = emptyList())

    /** Receives clipboard content applied by the background service. */
    public fun subscribeClipboard(onReceived: (ClipboardContent) -> Unit): AutoCloseable
}

/** Process-safe snapshot of the background Mist service. */
public data class MistControllerState(
    val endpointId: String = "",
    val devices: List<ManagedDevice> = emptyList(),
    val ready: Boolean = false,
    val startupError: String? = null,
)
