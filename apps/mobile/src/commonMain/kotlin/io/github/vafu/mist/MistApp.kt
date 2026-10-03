package io.github.vafu.mist

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.vafu.mist.core.DeviceInfo
import io.github.vafu.mist.core.MistRuntime
import io.github.vafu.mist.core.Pairing
import io.github.vafu.mist.core.PairingOffer
import io.github.vafu.mist.core.createMistRuntime
import kotlinx.coroutines.launch

/** Shared entry point for the Mist mobile user interface. */
@Composable
public fun MistApp(deviceInfo: DeviceInfo) {
    var runtime by remember { mutableStateOf<MistRuntime?>(null) }
    var pairing by remember { mutableStateOf<Pairing?>(null) }
    var pairingOffer by remember { mutableStateOf("") }
    var pairedPeer by remember { mutableStateOf<String?>(null) }
    var isPairing by remember { mutableStateOf(false) }
    var startupError by remember { mutableStateOf<String?>(null) }
    var pairingError by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(deviceInfo) {
        runCatching {
            val currentRuntime = createMistRuntime(deviceInfo)
            runtime = currentRuntime
            pairing = currentRuntime.pairing()
        }.onFailure {
            startupError = it.message ?: it::class.simpleName ?: "Mist runtime failed"
        }
    }
    val currentRuntime = runtime
    val currentPairing = pairing
    DisposableEffect(currentRuntime, currentPairing) {
        onDispose {
            currentPairing?.close()
            currentRuntime?.let { current ->
                scope.launch {
                    current.shutdown()
                    current.close()
                }
            }
        }
    }

    fun acceptPairingOffer(rawOffer: String) {
        val service = pairing
        val offer = rawOffer.trim()
        when {
            isPairing -> Unit
            service == null -> pairingError = "Mist runtime is not ready"
            !offer.startsWith("mist:pair:1:") -> pairingError = "This is not a Mist pairing QR code"
            else -> {
                pairingError = null
                isPairing = true
                scope.launch {
                    runCatching {
                        service.accept(PairingOffer(offer))
                    }.onSuccess { peer ->
                        pairedPeer = peer.device.displayName
                    }.onFailure {
                        pairingError = it.message ?: "Pairing failed"
                    }
                    isPairing = false
                }
            }
        }
    }

    val qrScanner = rememberQrScanner(
        onResult = ::acceptPairingOffer,
        onError = { pairingError = it },
    )

    MaterialTheme {
        Surface(Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier.padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Mist · Міст", style = MaterialTheme.typography.headlineMedium)
                when {
                    startupError != null -> {
                        Text("Iroh endpoint failed", color = MaterialTheme.colorScheme.error)
                        Text(startupError.orEmpty(), style = MaterialTheme.typography.bodySmall)
                    }
                    runtime == null -> {
                        CircularProgressIndicator()
                        Text("Starting the native Mist runtime…")
                    }
                    else -> {
                        Text("Mist runtime ready")
                        SelectionContainer {
                            Text(runtime!!.endpointId(), style = MaterialTheme.typography.bodySmall)
                        }
                        if (pairedPeer != null) {
                            Text("Paired with $pairedPeer")
                        } else {
                            Text("Scan the QR shown by the Mist CLI.")
                            Button(
                                enabled = currentPairing != null && !isPairing,
                                onClick = qrScanner::scan,
                            ) {
                                Text(if (isPairing) "Pairing…" else "Scan QR code")
                            }
                            Text("Debug fallback", style = MaterialTheme.typography.labelMedium)
                            OutlinedTextField(
                                value = pairingOffer,
                                onValueChange = { pairingOffer = it },
                                modifier = Modifier.fillMaxWidth(),
                                label = { Text("Pairing offer") },
                                enabled = !isPairing,
                            )
                            Button(
                                enabled = currentPairing != null && pairingOffer.isNotBlank() && !isPairing,
                                onClick = { acceptPairingOffer(pairingOffer) },
                            ) {
                                Text("Pair pasted offer")
                            }
                            if (isPairing) {
                                CircularProgressIndicator()
                            }
                            if (pairingError != null) {
                                Text(pairingError.orEmpty(), color = MaterialTheme.colorScheme.error)
                            }
                        }
                    }
                }
            }
        }
    }
}
