package io.github.vafu.mist

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.vafu.mist.core.Devices
import io.github.vafu.mist.core.MistRuntime
import io.github.vafu.mist.core.Pairing
import io.github.vafu.mist.core.PairingOffer
import io.github.vafu.mist.core.Peer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** UI adapter over the Rust runtime whose lifetime is owned by the host. */
@Composable
public fun MistApp(runtime: MistRuntime?, startupError: String?) {
    var pairing by remember { mutableStateOf<Pairing?>(null) }
    var devices by remember { mutableStateOf<Devices?>(null) }
    var peers by remember { mutableStateOf<List<Peer>>(emptyList()) }
    var offerText by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var actionError by remember { mutableStateOf<String?>(null) }
    var status by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(runtime) {
        pairing = null
        devices = null
        val current = runtime ?: return@LaunchedEffect
        val currentPairing = current.pairing()
        val currentDevices = current.devices()
        try {
            peers = withContext(Dispatchers.Default) { currentDevices.peers() }
            pairing = currentPairing
            devices = currentDevices
            awaitCancellation()
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (error: Exception) {
            actionError = error.message ?: "Device storage failed"
        } finally {
            currentPairing.close()
            currentDevices.close()
        }
    }

    fun perform(action: suspend () -> Unit) {
        if (busy) return
        busy = true
        actionError = null
        scope.launch {
            try {
                action()
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                actionError = error.message ?: "Device operation failed"
            } finally {
                busy = false
            }
        }
    }

    suspend fun refresh(service: Devices) {
        peers = withContext(Dispatchers.Default) { service.peers() }
    }

    fun acceptOffer(raw: String) {
        val service = pairing ?: return
        val saved = devices ?: return
        perform {
            val peer = service.accept(PairingOffer(raw.trim()))
            status = "Paired with ${peer.device.displayName}"
            refresh(saved)
        }
    }

    val scanner = rememberQrScanner(
        onResult = ::acceptOffer,
        onError = { actionError = it },
    )
    val current = runtime
    val saved = devices

    MaterialTheme {
        Surface(Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()).padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Mist · Міст", style = MaterialTheme.typography.headlineMedium)
                when {
                    startupError != null -> Text(startupError.orEmpty(), color = MaterialTheme.colorScheme.error)
                    actionError != null && saved == null -> Text(actionError.orEmpty(), color = MaterialTheme.colorScheme.error)
                    current == null || saved == null -> {
                        CircularProgressIndicator()
                        Text("Starting Mist…")
                    }
                    else -> {
                        SelectionContainer { Text(current.endpointId(), style = MaterialTheme.typography.bodySmall) }
                        Text("Saved devices", style = MaterialTheme.typography.titleLarge)
                        if (peers.isEmpty()) Text("No saved devices")
                        peers.forEach { peer ->
                            Text(peer.device.displayName, style = MaterialTheme.typography.titleMedium)
                            Text(peer.device.platform.toString())
                            SelectionContainer { Text(peer.endpointId, style = MaterialTheme.typography.bodySmall) }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                Button(enabled = !busy, onClick = {
                                    perform {
                                        current.reconnect(peer.endpointId)
                                        status = "Connected to ${peer.device.displayName}"
                                    }
                                }) { Text("Reconnect") }
                                Button(enabled = !busy, onClick = {
                                    perform {
                                        withContext(Dispatchers.Default) { saved.forget(peer.endpointId) }
                                        refresh(saved)
                                        status = "Forgot ${peer.device.displayName}"
                                    }
                                }) { Text("Forget") }
                            }
                        }
                        Button(enabled = !busy, onClick = { perform { refresh(saved) } }) { Text("Refresh saved devices") }
                        Text("Add a device", style = MaterialTheme.typography.titleLarge)
                        Button(enabled = !busy, onClick = scanner::scan) { Text("Scan pairing QR") }
                        OutlinedTextField(
                            value = offerText,
                            onValueChange = { offerText = it },
                            modifier = Modifier.fillMaxWidth(),
                            label = { Text("Pairing offer (debug fallback)") },
                            enabled = !busy,
                        )
                        Button(enabled = !busy && offerText.isNotBlank(), onClick = { acceptOffer(offerText) }) { Text("Pair pasted offer") }
                        if (busy) CircularProgressIndicator()
                        status?.let { Text(it) }
                        actionError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
                    }
                }
            }
        }
    }
}
