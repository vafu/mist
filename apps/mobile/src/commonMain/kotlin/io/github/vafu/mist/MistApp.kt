package io.github.vafu.mist

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.vafu.mist.core.DeviceStatus
import io.github.vafu.mist.core.ManagedDevice
import kotlinx.coroutines.launch

/** Shared entry point for the Mist mobile user interface. */
@Composable
public fun MistApp(
    controller: MistController,
    notificationPostingGranted: Boolean,
    notificationListenerGranted: Boolean,
    requestNotificationPosting: () -> Unit,
    requestNotificationListener: () -> Unit,
) {
    val state by controller.state.collectAsState()
    var pairingOffer by remember { mutableStateOf("") }
    var isPairing by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    fun acceptPairingOffer(rawOffer: String) {
        val offer = rawOffer.trim()
        when {
            isPairing -> Unit
            !offer.startsWith("mist:pair:1:") -> error = "This is not a Mist pairing QR code"
            else -> {
                error = null
                isPairing = true
                scope.launch {
                    runCatching { controller.acceptPairing(offer) }
                        .onFailure { error = it.message ?: "Pairing failed" }
                    isPairing = false
                }
            }
        }
    }

    val qrScanner = rememberQrScanner(
        onResult = ::acceptPairingOffer,
        onError = { error = it },
    )

    MaterialTheme {
        Surface(Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier.padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Mist · Міст", style = MaterialTheme.typography.headlineMedium)
                Text("Mist background service ready")
                SelectionContainer {
                    Text(state.endpointId, style = MaterialTheme.typography.bodySmall)
                }

                if (!notificationPostingGranted) {
                    Text("Android is not allowing Mist to show remote notifications.")
                    Button(onClick = requestNotificationPosting) {
                        Text("Allow Mist notifications")
                    }
                }

                if (!notificationListenerGranted) {
                    Text("Notification-listener access is required to forward local notifications.")
                    Button(onClick = requestNotificationListener) {
                        Text("Allow notification forwarding")
                    }
                }

                if (state.devices.isEmpty()) {
                    PairingControls(
                        pairingOffer = pairingOffer,
                        onPairingOfferChange = { pairingOffer = it },
                        isPairing = isPairing,
                        scan = qrScanner::scan,
                        pair = { acceptPairingOffer(pairingOffer) },
                    )
                } else {
                    Text("Devices", style = MaterialTheme.typography.titleMedium)
                    state.devices.forEach { device ->
                        DeviceControls(
                            device = device,
                            action = { operation ->
                                error = null
                                scope.launch {
                                    runCatching { operation() }
                                        .onFailure { error = it.message ?: "Device operation failed" }
                                }
                            },
                            select = { selected -> controller.selectDevice(device.endpointId, selected) },
                            connect = { controller.connectDevice(device.endpointId) },
                            disconnect = { controller.disconnectDevice(device.endpointId) },
                            forget = { controller.forgetDevice(device.endpointId) },
                        )
                    }
                    PairingControls(
                        pairingOffer = pairingOffer,
                        onPairingOfferChange = { pairingOffer = it },
                        isPairing = isPairing,
                        scan = qrScanner::scan,
                        pair = { acceptPairingOffer(pairingOffer) },
                    )
                }

                if (state.devices.isNotEmpty()) {
                    Button(
                        enabled = !isPairing,
                        onClick = {
                            error = null
                            scope.launch {
                                runCatching { controller.sendClipboard() }
                                    .onFailure { error = it.message ?: "Clipboard send failed" }
                            }
                        },
                    ) {
                        Text("Send clipboard")
                    }
                }

                if (isPairing) {
                    CircularProgressIndicator()
                }
                if (error != null) {
                    Text(error.orEmpty(), color = MaterialTheme.colorScheme.error)
                }
            }
        }
    }
}

@Composable
private fun PairingControls(
    pairingOffer: String,
    onPairingOfferChange: (String) -> Unit,
    isPairing: Boolean,
    scan: () -> Unit,
    pair: () -> Unit,
) {
    Text("Add device", style = MaterialTheme.typography.titleMedium)
    Button(enabled = !isPairing, onClick = scan) {
        Text(if (isPairing) "Pairing…" else "Scan QR code")
    }
    Text("Debug fallback", style = MaterialTheme.typography.labelMedium)
    OutlinedTextField(
        value = pairingOffer,
        onValueChange = onPairingOfferChange,
        modifier = Modifier.fillMaxWidth(),
        label = { Text("Pairing offer") },
        enabled = !isPairing,
    )
    Button(
        enabled = pairingOffer.isNotBlank() && !isPairing,
        onClick = pair,
    ) {
        Text("Pair pasted offer")
    }
}

@Composable
private fun DeviceControls(
    device: ManagedDevice,
    action: (suspend () -> Unit) -> Unit,
    select: suspend (Boolean) -> Unit,
    connect: suspend () -> Unit,
    disconnect: suspend () -> Unit,
    forget: suspend () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(device.device.displayName, style = MaterialTheme.typography.titleSmall)
        Text(
            "${device.device.deviceClass} · ${device.status} via ${device.transport}",
            style = MaterialTheme.typography.bodySmall,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { action { select(!device.selected) } }) {
                Text(if (device.selected) "Selected" else "Select")
            }
            if (device.status == DeviceStatus.CONNECTED) {
                Button(onClick = { action(disconnect) }) { Text("Disconnect") }
            } else {
                Button(onClick = { action(connect) }) { Text("Connect") }
            }
            Button(onClick = { action(forget) }) { Text("Forget") }
        }
    }
}
