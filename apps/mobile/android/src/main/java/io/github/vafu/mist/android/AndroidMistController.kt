package io.github.vafu.mist.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.net.Uri
import android.os.Bundle
import android.os.IBinder
import io.github.vafu.mist.MistController
import io.github.vafu.mist.MistControllerState
import io.github.vafu.mist.core.ClipboardContent
import io.github.vafu.mist.core.ConnectionTransport
import io.github.vafu.mist.core.DeviceClass
import io.github.vafu.mist.core.DeviceInfo
import io.github.vafu.mist.core.DeviceStatus
import io.github.vafu.mist.core.ManagedDevice
import io.github.vafu.mist.core.Platform
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Binder-backed Android implementation of the shared mobile controller. */
public class AndroidMistController(context: Context) : MistController {
    private val applicationContext: Context = context.applicationContext
    private val remoteService = MutableStateFlow<IMistService?>(null)
    private val mutableState = MutableStateFlow(MistControllerState())

    override val state: StateFlow<MistControllerState> = mutableState.asStateFlow()

    private val clipboardReceivers = java.util.concurrent.CopyOnWriteArrayList<(Bundle) -> Unit>()
    private val actionReceivers = java.util.concurrent.CopyOnWriteArrayList<(Bundle) -> Unit>()

    private val callback: IMistCallback = object : IMistCallback.Stub() {
        override fun onStateChanged(state: Bundle) {
            mutableState.value = state.toControllerState()
        }

        override fun onClipboardReceived(content: Bundle) {
            clipboardReceivers.forEach { receiver -> runCatching { receiver(content) } }
        }

        override fun onNotificationAction(activation: Bundle) {
            actionReceivers.forEach { receiver -> runCatching { receiver(activation) } }
        }
    }

    private val connection: ServiceConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val remote = IMistService.Stub.asInterface(binder)
            remoteService.value = remote
            runCatching {
                remote.registerCallback(callback)
                mutableState.value = remote.snapshot().toControllerState()
            }.onFailure { error ->
                mutableState.value = MistControllerState(startupError = error.message)
            }
        }

        override fun onServiceDisconnected(name: ComponentName) {
            remoteService.value = null
            mutableState.value = mutableState.value.copy(
                ready = false,
                startupError = "Mist background service disconnected",
            )
        }
    }

    init {
        check(applicationContext.bindService(connectionIntent(), connection, Context.BIND_AUTO_CREATE)) {
            "Could not bind the Mist background service"
        }
    }

    override suspend fun initiatePairing(): String =
        request { remote, callback -> remote.initiatePairing(callback) }.getString(KEY_VALUE).orEmpty()

    override fun cancelPairing() {
        runCatching { remoteService.value?.cancelPairing() }
    }

    override suspend fun acceptPairing(offer: String) {
        request { remote, callback -> remote.acceptPairing(offer, callback) }
    }

    override suspend fun selectDevice(endpointId: String, selected: Boolean) {
        request { remote, callback -> remote.selectDevice(endpointId, selected, callback) }
    }

    override suspend fun connectDevice(endpointId: String) {
        request { remote, callback -> remote.connectDevice(endpointId, callback) }
    }

    override suspend fun disconnectDevice(endpointId: String) {
        request { remote, callback -> remote.disconnectDevice(endpointId, callback) }
    }

    override suspend fun forgetDevice(endpointId: String) {
        request { remote, callback -> remote.forgetDevice(endpointId, callback) }
    }

    override suspend fun sendClipboard(targets: List<String>) {
        val content = readClipboard() ?: return
        pushClipboard(content, targets)
    }

    private suspend fun pushClipboard(content: ClipboardContent, targets: List<String>) {
        request { remote, callback ->
            remote.pushClipboard(content.toBundle(), targets.toTypedArray(), callback)
        }
    }

    override fun subscribeClipboard(onReceived: (ClipboardContent) -> Unit): AutoCloseable {
        val receiver: (Bundle) -> Unit = { bundle ->
            val content = bundle.getBundle(KEY_CLIPBOARD_CONTENT)?.toClipboardContent()
            if (content != null) onReceived(content)
        }
        clipboardReceivers.add(receiver)
        return AutoCloseable { clipboardReceivers.remove(receiver) }
    }

    private suspend fun readClipboard(): ClipboardContent? = withContext(Dispatchers.Main) {
        val manager = applicationContext.getSystemService(ClipboardManager::class.java)
        val clip = manager.primaryClip ?: return@withContext null
        if (clip.itemCount == 0) return@withContext null
        val item = clip.getItemAt(0)
        val uri = item.uri
        if (uri != null) {
            val mimeType = applicationContext.contentResolver.getType(uri) ?: "application/octet-stream"
            if (mimeType.startsWith("image/")) {
                val file = copyToCache(uri, mimeType) ?: return@withContext null
                return@withContext ClipboardContent.Blob(mimeType, file.absolutePath)
            }
        }
        item.coerceToText(applicationContext).toString().takeIf { it.isNotBlank() }?.let {
            ClipboardContent.Text(it)
        }
    }

    /** Registers a listener for actions activated on this device's notifications. */
    public fun subscribeNotificationActions(onActivated: (String, String, String) -> Unit): AutoCloseable {
        val receiver: (Bundle) -> Unit = { bundle ->
            val endpoint = bundle.getString(KEY_ACTIVATION_ENDPOINT).orEmpty()
            val notification = bundle.getString(KEY_ACTIVATION_NOTIFICATION).orEmpty()
            val action = bundle.getString(KEY_ACTIVATION_ACTION).orEmpty()
            if (endpoint.isNotEmpty() && notification.isNotEmpty() && action.isNotEmpty()) {
                onActivated(endpoint, notification, action)
            }
        }
        actionReceivers.add(receiver)
        return AutoCloseable { actionReceivers.remove(receiver) }
    }

    override fun close() {
        runCatching { remoteService.value?.unregisterCallback(callback) }
        runCatching { applicationContext.unbindService(connection) }
    }

    private suspend fun copyToCache(uri: Uri, mimeType: String): File? =
        withContext(Dispatchers.IO) {
            runCatching {
                val directory = File(applicationContext.cacheDir, "clipboard/out").apply { mkdirs() }
                val path = File(
                    directory,
                    "clipboard-${System.currentTimeMillis()}.${extension(mimeType)}",
                )
                applicationContext.contentResolver.openInputStream(uri)?.use { input ->
                    path.outputStream().use { output -> input.copyTo(output) }
                } ?: return@withContext null
                path
            }.getOrNull()
        }

    private fun extension(mimeType: String): String =
        when (mimeType) {
            "image/png" -> "png"
            "image/jpeg" -> "jpg"
            "image/webp" -> "webp"
            "image/gif" -> "gif"
            else -> "bin"
        }

    private fun connectionIntent(): Intent =
        Intent(applicationContext, MistRuntimeService::class.java).setAction(MistServiceStarter.ACTION_START)

    private suspend fun request(
        send: (IMistService, IMistResultCallback) -> Unit,
    ): Bundle {
        val remote = remoteService.filterNotNull().first()
        return suspendCancellableCoroutine { continuation ->
            val result = object : IMistResultCallback.Stub() {
                override fun onSuccess(result: Bundle) {
                    if (continuation.isActive) continuation.resume(result)
                }

                override fun onFailure(message: String) {
                    if (continuation.isActive) {
                        continuation.resumeWithException(IllegalStateException(message))
                    }
                }
            }
            runCatching { send(remote, result) }
                .onFailure { error ->
                    if (continuation.isActive) continuation.resumeWithException(error)
                }
        }
    }
}

private fun Bundle.toControllerState(): MistControllerState {
    @Suppress("DEPRECATION")
    val deviceBundles = getParcelableArrayList<Bundle>(KEY_DEVICES).orEmpty()
    return MistControllerState(
        endpointId = getString(KEY_ENDPOINT_ID).orEmpty(),
        devices = deviceBundles.map(Bundle::toManagedDevice),
        ready = getBoolean(KEY_READY),
        startupError = getString(KEY_STARTUP_ERROR),
    )
}

private fun Bundle.toManagedDevice(): ManagedDevice = ManagedDevice(
    endpointId = getString("endpoint_id").orEmpty(),
    device = DeviceInfo(
        displayName = getString("display_name").orEmpty(),
        deviceClass = DeviceClass.valueOf(getString("device_class").orEmpty()),
        platform = Platform.valueOf(getString("platform").orEmpty()),
    ),
    status = DeviceStatus.valueOf(getString("status").orEmpty()),
    transport = ConnectionTransport.valueOf(getString("transport").orEmpty()),
    selected = getBoolean("selected"),
)
