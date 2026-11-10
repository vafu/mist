package io.github.vafu.mist

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.vafu.mist.core.ConnectionTransport
import io.github.vafu.mist.core.DeviceStatus
import io.github.vafu.mist.core.ManagedDevice
import io.github.vafu.mist.ui.AppMark
import io.github.vafu.mist.ui.CameraIcon
import io.github.vafu.mist.ui.CheckIcon
import io.github.vafu.mist.ui.ClipboardIcon
import io.github.vafu.mist.ui.PlusIcon
import io.github.vafu.mist.ui.ShareIcon
import io.github.vafu.mist.ui.TrashIcon
import io.github.vafu.mist.ui.MistTheme
import kotlinx.coroutines.launch

/** Shared entry point for the Mist mobile user interface. */
@Composable
public fun MistApp(
    controller: MistController,
    notificationPostingGranted: Boolean,
    notificationListenerGranted: Boolean,
    requestNotificationPosting: () -> Unit,
    requestNotificationListener: () -> Unit,
    pickAndShareFile: () -> Unit,
) {
    MistTheme {
        MistScreen(
            controller = controller,
            notificationPostingGranted = notificationPostingGranted,
            notificationListenerGranted = notificationListenerGranted,
            requestNotificationPosting = requestNotificationPosting,
            requestNotificationListener = requestNotificationListener,
            pickAndShareFile = pickAndShareFile,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MistScreen(
    controller: MistController,
    notificationPostingGranted: Boolean,
    notificationListenerGranted: Boolean,
    requestNotificationPosting: () -> Unit,
    requestNotificationListener: () -> Unit,
    pickAndShareFile: () -> Unit,
) {
    val state by controller.state.collectAsState()
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    var showAddDevice by remember { mutableStateOf(false) }
    var pendingAction by remember { mutableStateOf<String?>(null) }

    val permissionsComplete = notificationPostingGranted && notificationListenerGranted
    val connected = state.devices.any { it.status == DeviceStatus.CONNECTED }

    fun runAction(label: String, block: suspend () -> Unit) {
        pendingAction = label
        scope.launch {
            runCatching { block() }
                .onFailure { snackbar.showSnackbar(it.message ?: "Something went wrong") }
            pendingAction = null
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        AppMark(size = 32.dp)
                        Spacer(Modifier.size(12.dp))
                        Text("Mist", style = MaterialTheme.typography.titleLarge)
                    }
                },
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
        floatingActionButton = {
            if (state.devices.isNotEmpty()) {
                ExtendedFloatingActionButton(
                    text = { Text("Add device") },
                    icon = { PlusIcon() },
                    onClick = { showAddDevice = true },
                )
            }
        },
    ) { padding ->
        when {
            state.startupError != null -> StartupError(state.startupError.orEmpty(), padding)
            !state.ready -> Loading(padding)
            else -> LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
                contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 96.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (pendingAction != null) {
                    item { LinearProgressIndicator(Modifier.fillMaxWidth()) }
                }

                if (!permissionsComplete) {
                    item {
                        PermissionsCard(
                            posting = notificationPostingGranted,
                            listener = notificationListenerGranted,
                            requestPosting = requestNotificationPosting,
                            requestListener = requestNotificationListener,
                        )
                    }
                }

                if (state.devices.isEmpty()) {
                    item {
                        EmptyState(onAdd = { showAddDevice = true })
                    }
                } else {
                    item {
                        SectionHeader(
                            title = "Devices",
                            subtitle = if (connected) "Connected" else "Not connected",
                        )
                    }
                    items(state.devices, key = { it.endpointId }) { device ->
                        DeviceCard(
                            device = device,
                            busy = pendingAction != null,
                            onToggleSelected = {
                                runAction("Updating") {
                                    controller.selectDevice(device.endpointId, !device.selected)
                                }
                            },
                            onToggleConnection = {
                                runAction("Connecting") {
                                    if (device.status == DeviceStatus.CONNECTED) {
                                        controller.disconnectDevice(device.endpointId)
                                    } else {
                                        controller.connectDevice(device.endpointId)
                                    }
                                }
                            },
                            onForget = {
                                runAction("Removing") {
                                    controller.forgetDevice(device.endpointId)
                                }
                            },
                        )
                    }

                    item {
                        SectionHeader(title = "Share", subtitle = null)
                    }
                    item {
                        ShareCard(
                            enabled = pendingAction == null,
                            onSendClipboard = {
                                runAction("Sending clipboard") { controller.sendClipboard() }
                            },
                            onSendFile = pickAndShareFile,
                        )
                    }
                }
            }
        }
    }

    if (showAddDevice) {
        AddDeviceSheet(
            controller = controller,
            onDismiss = { showAddDevice = false },
            onError = { message -> scope.launch { snackbar.showSnackbar(message) } },
        )
    }
}

@Composable
private fun SectionHeader(title: String, subtitle: String?) {
    Column(Modifier.padding(top = 8.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium)
        if (subtitle != null) {
            Text(
                subtitle,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun Loading(padding: PaddingValues) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            CircularProgressIndicator()
            Spacer(Modifier.height(16.dp))
            Text(
                "Starting Mist…",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun StartupError(message: String, padding: PaddingValues) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .padding(padding)
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            AppMark(size = 56.dp, tint = MaterialTheme.colorScheme.error)
            Spacer(Modifier.height(16.dp))
            Text("Mist couldn't start", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(8.dp))
            Text(
                message,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun EmptyState(onAdd: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 48.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        AppMark(size = 88.dp)
        Spacer(Modifier.height(20.dp))
        Text("No devices yet", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(8.dp))
        Text(
            "Pair a phone or computer to share notifications, clipboard and files.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 24.dp),
        )
        Spacer(Modifier.height(24.dp))
        Button(onClick = onAdd) { Text("Add a device") }
    }
}

@Composable
private fun PermissionsCard(
    posting: Boolean,
    listener: Boolean,
    requestPosting: () -> Unit,
    requestListener: () -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
        ),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                "Finish setting up",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
            if (!posting) {
                PermissionRow(
                    title = "Show notifications",
                    description = "Display alerts sent from your other devices.",
                    action = "Allow",
                    onAction = requestPosting,
                )
            }
            if (!listener) {
                if (!posting) HorizontalDivider(Modifier.padding(vertical = 4.dp))
                PermissionRow(
                    title = "Forward your notifications",
                    description = "Send this device's notifications to your other devices.",
                    action = "Open settings",
                    onAction = requestListener,
                )
            }
        }
    }
}

@Composable
private fun PermissionRow(
    title: String,
    description: String,
    action: String,
    onAction: () -> Unit,
) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(
                title,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
            Text(
                description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        }
        TextButton(onClick = onAction) { Text(action) }
    }
}

@Composable
private fun DeviceCard(
    device: ManagedDevice,
    busy: Boolean,
    onToggleSelected: () -> Unit,
    onToggleConnection: () -> Unit,
    onForget: () -> Unit,
) {
    val connected = device.status == DeviceStatus.CONNECTED
    Card {
        Column {
            Row(
                modifier = Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                DeviceAvatar(device, connected)
                Column(Modifier.weight(1f).padding(start = 12.dp)) {
                    Text(
                        device.device.displayName,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        statusLine(device),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Switch(
                    checked = connected,
                    onCheckedChange = { onToggleConnection() },
                    enabled = !busy && device.status != DeviceStatus.CONNECTING,
                )
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 16.dp, end = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                TextButton(onClick = onToggleSelected, enabled = !busy) {
                    if (device.selected) {
                        CheckIcon(size = 18.dp)
                        Spacer(Modifier.size(6.dp))
                    }
                    Text(if (device.selected) "Destination" else "Set destination")
                }
                Spacer(Modifier.weight(1f))
                IconButton(onClick = onForget, enabled = !busy) {
                    TrashIcon()
                }
            }
        }
    }
}

@Composable
private fun DeviceAvatar(device: ManagedDevice, connected: Boolean) {
    Box(
        modifier = Modifier
            .size(44.dp)
            .clip(MaterialTheme.shapes.medium)
            .background(
                if (connected) {
                    MaterialTheme.colorScheme.tertiaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceContainerHighest
                },
            ),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            device.device.displayName.take(1).uppercase(),
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.SemiBold,
            color = if (connected) {
                MaterialTheme.colorScheme.onTertiaryContainer
            } else {
                MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
    }
}

private fun statusLine(device: ManagedDevice): String = when (device.status) {
    DeviceStatus.CONNECTED -> "Connected · ${transportLabel(device.transport)}"
    DeviceStatus.CONNECTING -> "Connecting…"
    DeviceStatus.DISCONNECTED -> "Not connected"
}

private fun transportLabel(transport: ConnectionTransport): String = when (transport) {
    ConnectionTransport.DIRECT -> "direct"
    ConnectionTransport.RELAY -> "relay"
    ConnectionTransport.CUSTOM -> "custom"
    ConnectionTransport.UNKNOWN -> "unknown"
}

@Composable
private fun ShareCard(
    enabled: Boolean,
    onSendClipboard: () -> Unit,
    onSendFile: () -> Unit,
) {
    Card {
        Column {
            ListItem(
                headlineContent = { Text("Send clipboard") },
                supportingContent = {
                    Text("Share what you copied with your selected devices.")
                },
                leadingContent = { ClipboardIcon() },
                trailingContent = {
                    TextButton(onClick = onSendClipboard, enabled = enabled) { Text("Send") }
                },
            )
            HorizontalDivider()
            ListItem(
                headlineContent = { Text("Send a file") },
                supportingContent = { Text("Pick a file to send to your selected devices.") },
                leadingContent = { ShareIcon() },
                trailingContent = {
                    TextButton(onClick = onSendFile, enabled = enabled) { Text("Choose") }
                },
            )
        }
    }
}


@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AddDeviceSheet(
    controller: MistController,
    onDismiss: () -> Unit,
    onError: (String) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var busy by remember { mutableStateOf(false) }

    fun accept(raw: String) {
        val offer = raw.trim()
        if (busy) return
        if (!offer.startsWith("mist:pair:1:")) {
            onError("That QR code isn't a Mist pairing code")
            return
        }
        busy = true
        scope.launch {
            runCatching { controller.acceptPairing(offer) }
                .onSuccess { onDismiss() }
                .onFailure { onError(it.message ?: "Pairing failed") }
            busy = false
        }
    }

    val scanner = rememberQrScanner(
        onResult = ::accept,
        onError = { onError(it) },
    )

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheetState) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 24.dp, end = 24.dp, bottom = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            AppMark(size = 64.dp)
            Text("Pair a device", style = MaterialTheme.typography.headlineSmall)
            Text(
                "On your other device, open Mist and show its pairing QR code. Then scan it here.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))
            if (busy) {
                CircularProgressIndicator()
                Text(
                    "Pairing…",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Button(onClick = scanner::scan, modifier = Modifier.fillMaxWidth()) {
                    CameraIcon()
                    Spacer(Modifier.size(8.dp))
                    Text("Scan QR code")
                }
            }
            TextButton(onClick = onDismiss, enabled = !busy) { Text("Cancel") }
        }
    }
}
