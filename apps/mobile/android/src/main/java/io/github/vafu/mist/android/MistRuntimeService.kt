package io.github.vafu.mist.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.os.RemoteCallbackList
import android.util.Log
import io.github.vafu.mist.core.ClipboardContent
import io.github.vafu.mist.core.Clipboard
import io.github.vafu.mist.core.DeviceClass
import io.github.vafu.mist.core.DeviceInfo
import io.github.vafu.mist.core.DeviceManager
import io.github.vafu.mist.core.Media
import io.github.vafu.mist.core.MediaCommandHandler
import io.github.vafu.mist.core.MistApp
import io.github.vafu.mist.core.Notification
import io.github.vafu.mist.core.NotificationImage
import io.github.vafu.mist.core.NotificationKind
import io.github.vafu.mist.core.NotificationChannel as MistNotificationChannel
import io.github.vafu.mist.core.Pairing
import io.github.vafu.mist.core.PairingOffer
import io.github.vafu.mist.core.Platform
import io.github.vafu.mist.host.createMistAppWithMedia
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** Owns the only Android Rust runtime, in the dedicated `:mist` process. */
public class MistRuntimeService : Service() {
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val runtimeReady = kotlinx.coroutines.CompletableDeferred<Clipboard?>()
    private val callbacks: RemoteCallbackList<IMistCallback> = RemoteCallbackList()

    @Volatile
    private var currentSnapshot: Bundle = stateBundle()
    private var app: MistApp? = null
    private var devices: DeviceManager? = null
    private var pairing: Pairing? = null
    private var notifications: MistNotificationChannel? = null
    private var clipboard: Clipboard? = null
    private var media: Media? = null
    private var androidMedia: AndroidMedia? = null
    private var mediaSession: MistMediaSession? = null

    private val mediaHandler: MediaCommandHandler = object : MediaCommandHandler {
        override fun command(
            playerId: String,
            command: io.github.vafu.mist.core.MediaCommand,
        ) {
            runBlocking(Dispatchers.Default) {
                runCatching { androidMedia?.command(playerId, command) }
                    .onFailure { Log.e(TAG, "media command failed", it) }
            }
        }
    }

    private val binder: IMistService.Stub = object : IMistService.Stub() {
        override fun snapshot(): Bundle = Bundle(currentSnapshot)

        override fun registerCallback(callback: IMistCallback) {
            callbacks.register(callback)
            runCatching { callback.onStateChanged(Bundle(currentSnapshot)) }
        }

        override fun unregisterCallback(callback: IMistCallback) {
            callbacks.unregister(callback)
        }

        override fun initiatePairing(callback: IMistResultCallback) {
            runOperation(callback) {
                val service = requirePairing()
                val offer = service.initiate().encoded
                scope.launch {
                    runCatching { service.waitForPeer() }
                        .onFailure { Log.e(TAG, "Incoming pairing failed", it) }
                }
                Bundle().apply { putString(KEY_VALUE, offer) }
            }
        }

        override fun cancelPairing() {
            pairing?.cancel()
        }

        override fun acceptPairing(offer: String, callback: IMistResultCallback) {
            runOperation(callback) {
                requirePairing().accept(PairingOffer(offer))
                Bundle.EMPTY
            }
        }

        override fun selectDevice(
            endpointId: String,
            selected: Boolean,
            callback: IMistResultCallback,
        ) {
            runOperation(callback) {
                requireDevices().select(endpointId, selected)
                publishSnapshot()
                Bundle.EMPTY
            }
        }

        override fun connectDevice(endpointId: String, callback: IMistResultCallback) {
            runOperation(callback) {
                requireDevices().connect(endpointId)
                Bundle.EMPTY
            }
        }

        override fun disconnectDevice(endpointId: String, callback: IMistResultCallback) {
            runOperation(callback) {
                requireDevices().disconnect(endpointId)
                publishSnapshot()
                Bundle.EMPTY
            }
        }

        override fun forgetDevice(endpointId: String, callback: IMistResultCallback) {
            runOperation(callback) {
                requireDevices().forget(endpointId)
                publishSnapshot()
                Bundle.EMPTY
            }
        }

        override fun pushClipboard(
            content: Bundle,
            targets: Array<String>,
            callback: IMistResultCallback,
        ) {
            runOperation(callback) {
                requireClipboard().push(
                    content.toClipboardContent(),
                    targets.toList(),
                )
                Bundle.EMPTY
            }
        }

        override fun pushNotification(notification: Bundle) {
            scope.launch {
                runCatching {
                    requireNotifications().push(notification.toMistNotification(), emptyList())
                }.onFailure { Log.e(TAG, "Could not forward Android notification", it) }
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannels()
        startForeground(FOREGROUND_NOTIFICATION_ID, foregroundNotification())
        scope.launch { startRuntime() }
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == MistServiceStarter.ACTION_STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForeground(FOREGROUND_NOTIFICATION_ID, foregroundNotification())
        intent?.let(MistServiceStarter::sharePayload)?.let { payload ->
            scope.launch { sendShare(payload) }
        }
        if (intent?.action == MistServiceStarter.ACTION_MEDIA_COMMAND) {
            val command = intent.getStringExtra(MistServiceStarter.EXTRA_MEDIA_COMMAND)
            dispatchMediaCommand(command)
        }
        return START_STICKY
    }

    /**
     * Sends share-sheet content once the runtime is ready.
     *
     * A share can cold-start this service, so readiness is awaited rather than
     * assumed; failures are logged instead of being silently dropped.
     */
    private suspend fun sendShare(payload: SharePayload) {
        val clipboard = runtimeReady.await()
        if (clipboard == null) {
            Log.e(TAG, "Mist runtime failed to start; dropping share")
            return
        }
        val content = when (payload) {
            is SharePayload.Text -> ClipboardContent.Text(payload.text)
            is SharePayload.Image -> {
                val file = copySharedImage(payload.uri, payload.mimeType)
                if (file == null) {
                    Log.e(TAG, "Could not read shared image")
                    return
                }
                ClipboardContent.Blob(payload.mimeType, file.absolutePath)
            }
        }
        runCatching {
            clipboard.push(content, emptyList())
        }.onFailure { error ->
            Log.e(TAG, "Could not send shared content", error)
        }
    }

    private suspend fun copySharedImage(uri: android.net.Uri, mimeType: String): File? =
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            runCatching {
                val directory = File(cacheDir, "clipboard/out").apply { mkdirs() }
                val target = File(directory, "share-${System.currentTimeMillis()}.${extension(mimeType)}")
                contentResolver.openInputStream(uri)?.use { input ->
                    target.outputStream().use { output -> input.copyTo(output) }
                } ?: return@withContext null
                target
            }.getOrNull()
        }

    override fun onTaskRemoved(rootIntent: Intent?) {
        Log.i(TAG, "Mist task removed; keeping the runtime service alive")
    }

    override fun onDestroy() {
        callbacks.kill()
        val currentApp = app
        if (currentApp != null) {
            runBlocking(Dispatchers.Default) { runCatching { currentApp.shutdown() } }
        }
        closeRuntimeHandles()
        scope.cancel()
        super.onDestroy()
    }

    private suspend fun startRuntime() {
        runCatching {
            createMistAppWithMedia(
                filesDir.resolve("mist").absolutePath,
                DeviceInfo(
                    displayName = Build.MODEL,
                    deviceClass = DeviceClass.PHONE,
                    platform = Platform.ANDROID,
                ),
                mediaHandler,
            )
        }.onSuccess { runtime ->
            app = runtime
            devices = runtime.deviceManager()
            pairing = runtime.pairing()
            notifications = runtime.notificationChannel()
            clipboard = runtime.clipboard()
            media = runtime.media()
            runtimeReady.complete(clipboard)
            publishSnapshot()
            scope.launch { forwardDeviceEvents() }
            scope.launch { receiveRemoteNotifications() }
            scope.launch { receiveClipboard() }
            androidMedia = AndroidMedia(this@MistRuntimeService).also { it.start() }
            mediaSession = MistMediaSession(this@MistRuntimeService) { playerId, command ->
                scope.launch {
                    runCatching { runtime.media().command(playerId, command) }
                        .onFailure { Log.e(TAG, "Could not send media command", it) }
                }
            }.also { it.start() }
            scope.launch { forwardMediaEvents() }
        }.onFailure { error ->
            Log.e(TAG, "Mist runtime failed to start", error)
            runtimeReady.complete(null)
            currentSnapshot = stateBundle(startupError = error.message ?: "Mist startup failed")
            notifyCallbacks()
        }
    }

    private suspend fun forwardDeviceEvents() {
        val stream = requireDevices().subscribe()
        try {
            while (true) {
                stream.next()
                publishSnapshot()
            }
        } finally {
            stream.close()
        }
    }

    private suspend fun receiveRemoteNotifications() {
        val stream = requireNotifications().subscribe()
        try {
            while (true) {
                val received = stream.next()
                val channelId = if (received.notification.silent) {
                    REMOTE_SILENT_CHANNEL_ID
                } else {
                    REMOTE_CHANNEL_ID
                }
                val builder = android.app.Notification.Builder(this, channelId)
                    .setSmallIcon(android.R.drawable.ic_dialog_info)
                    .setContentTitle(received.notification.title)
                    .setContentText(received.notification.body)
                    .setSubText(
                        "${received.notification.application} · ${received.sourceDevice.displayName}",
                    )
                    .setAutoCancel(true)
                val icon = received.notification.appIcon?.loadBitmap()
                val image = received.notification.image?.loadBitmap()
                if (icon != null) {
                    builder.setLargeIcon(icon)
                }
                if (image != null) {
                    builder.setLargeIcon(image)
                    builder.setStyle(
                        android.app.Notification.BigPictureStyle().bigPicture(image),
                    )
                }
                val notification = builder.build()
                runCatching {
                    getSystemService(NotificationManager::class.java).notify(
                        "${received.sourceEndpointId}:${received.notification.id}",
                        0,
                        notification,
                    )
                }.onFailure { Log.e(TAG, "Could not show remote notification", it) }
            }
        } finally {
            stream.close()
        }
    }

    private suspend fun receiveClipboard() {
        val stream = requireClipboard().subscribe()
        try {
            while (true) {
                val received = stream.next()
                applyClipboard(received.content)
                notifyClipboardCallbacks(received)
            }
        } finally {
            stream.close()
        }
    }

    private fun applyClipboard(content: ClipboardContent) {
        val manager = getSystemService(ClipboardManager::class.java)
        when (content) {
            is ClipboardContent.Text -> {
                manager.setPrimaryClip(ClipData.newPlainText("Mist", content.text))
            }
            is ClipboardContent.Blob -> {
                val source = File(content.path)
                if (!source.isFile) {
                    Log.e(TAG, "Received clipboard blob is missing")
                    return
                }
                val inbox = File(filesDir, "clipboard/inbox").apply { mkdirs() }
                val target = File(inbox, "clipboard-${System.currentTimeMillis()}.${extension(content.mimeType)}")
                source.copyTo(target, overwrite = true)
                val uri = androidx.core.content.FileProvider.getUriForFile(
                    this,
                    "$packageName.clipboard",
                    target,
                )
                manager.setPrimaryClip(ClipData.newUri(contentResolver, "Mist", uri))
            }
        }
    }

    private fun notifyClipboardCallbacks(received: io.github.vafu.mist.core.ReceivedClipboard) {
        val count = callbacks.beginBroadcast()
        try {
            repeat(count) { index ->
                runCatching {
                    callbacks.getBroadcastItem(index).onClipboardReceived(received.toBundle())
                }
            }
        } finally {
            callbacks.finishBroadcast()
        }
    }

    /** Forwards media player changes into the local media session. */
    private suspend fun forwardMediaEvents() {
        val media = media ?: return
        val stream = media.subscribe()
        try {
            // Seed the session with the current player set.
            mediaSession?.update(media.list())
            while (true) {
                stream.next()
                mediaSession?.update(media.list())
            }
        } finally {
            stream.close()
        }
    }

    private fun dispatchMediaCommand(raw: String?) {
        val command = runCatching {
            io.github.vafu.mist.core.MediaCommand.valueOf(raw.orEmpty())
        }.getOrNull() ?: return
        val playerId = mediaSession?.remotePlayerId() ?: return
        scope.launch {
            runCatching { media?.command(playerId, command) }
                .onFailure { Log.e(TAG, "media command from notification failed", it) }
        }
    }

    /** Called by [AndroidMedia] whenever platform playback state changes. */
    internal fun onLocalMediaChanged(players: List<LocalMediaPlayer>) {
        val media = media ?: return
        runCatching {
            media.publishLocal(
                players.map { player ->
                    io.github.vafu.mist.core.MediaPlayer(
                        id = player.id,
                        endpointId = "",
                        name = player.name,
                        device = DeviceInfo(
                            displayName = Build.MODEL,
                            deviceClass = DeviceClass.PHONE,
                            platform = Platform.ANDROID,
                        ),
                        origin = io.github.vafu.mist.core.MediaOrigin.LOCAL,
                        status = player.status,
                        metadata = player.metadata,
                        capabilities = player.capabilities,
                        positionMs = player.positionMs,
                    )
                },
            )
        }.onFailure { Log.e(TAG, "Could not publish local media", it) }
    }

    private fun publishSnapshot() {
        val runtime = app ?: return
        val manager = devices ?: return
        currentSnapshot = stateBundle(runtime.endpointId(), manager.list())
        notifyCallbacks()
    }

    @Synchronized
    private fun notifyCallbacks() {
        val count = callbacks.beginBroadcast()
        try {
            repeat(count) { index ->
                runCatching { callbacks.getBroadcastItem(index).onStateChanged(Bundle(currentSnapshot)) }
            }
        } finally {
            callbacks.finishBroadcast()
        }
    }

    private fun runOperation(
        callback: IMistResultCallback,
        operation: suspend () -> Bundle,
    ) {
        scope.launch {
            runCatching { operation() }
                .onSuccess { runCatching { callback.onSuccess(it) } }
                .onFailure { error ->
                    runCatching { callback.onFailure(error.message ?: "Mist operation failed") }
                }
        }
    }

    private fun requireDevices(): DeviceManager =
        devices ?: error("Mist runtime is not ready")

    private fun requirePairing(): Pairing = pairing ?: error("Mist runtime is not ready")

    private fun requireNotifications(): MistNotificationChannel =
        notifications ?: error("Mist runtime is not ready")

    private fun requireClipboard(): Clipboard =
        clipboard ?: error("Mist runtime is not ready")

    private fun requireMedia(): Media = media ?: error("Mist runtime is not ready")

    private fun createNotificationChannels() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                SERVICE_CHANNEL_ID,
                "Mist background service",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
        manager.createNotificationChannel(
            NotificationChannel(
                REMOTE_CHANNEL_ID,
                "Notifications from Mist devices",
                NotificationManager.IMPORTANCE_DEFAULT,
            ),
        )
        manager.createNotificationChannel(
            NotificationChannel(
                REMOTE_SILENT_CHANNEL_ID,
                "Silent notifications from Mist devices",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
    }

    private fun foregroundNotification(): android.app.Notification =
        android.app.Notification.Builder(this, SERVICE_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_data_bluetooth)
            .setContentTitle("Mist is running")
            .setContentText("Keeping your paired devices connected")
            .setOngoing(true)
            .build()

    private fun closeRuntimeHandles() {
        mediaSession?.release()
        mediaSession = null
        androidMedia?.stop()
        media?.close()
        clipboard?.close()
        notifications?.close()
        pairing?.close()
        devices?.close()
        app?.close()
        notifications = null
        pairing = null
        devices = null
        app = null
    }

    private companion object {
        const val TAG: String = "MistRuntimeService"

        const val FOREGROUND_NOTIFICATION_ID: Int = 1
        const val SERVICE_CHANNEL_ID: String = "mist_service"
        const val REMOTE_CHANNEL_ID: String = "mist_remote_notifications"
        const val REMOTE_SILENT_CHANNEL_ID: String = "mist_remote_notifications_silent"
    }
}

internal const val KEY_READY: String = "ready"
internal const val KEY_ENDPOINT_ID: String = "endpoint_id"
internal const val KEY_DEVICES: String = "devices"
internal const val KEY_STARTUP_ERROR: String = "startup_error"
internal const val KEY_VALUE: String = "value"
internal const val KEY_ID: String = "id"
internal const val KEY_APPLICATION: String = "application"
internal const val KEY_TITLE: String = "title"
internal const val KEY_BODY: String = "body"
internal const val KEY_TIMESTAMP_MS: String = "timestamp_ms"
internal const val KEY_SILENT: String = "silent"
internal const val KEY_NOTIFICATION_KIND: String = "notification_kind"
internal const val KEY_APP_ICON: String = "notification_app_icon"
internal const val KEY_IMAGE: String = "notification_image"
internal const val KEY_IMAGE_MIME_TYPE: String = "image_mime_type"
internal const val KEY_IMAGE_PATH: String = "image_path"
internal const val KEY_CLIPBOARD_KIND: String = "clipboard_kind"
internal const val KEY_CLIPBOARD_TEXT: String = "clipboard_text"
internal const val KEY_CLIPBOARD_MIME_TYPE: String = "clipboard_mime_type"
internal const val KEY_CLIPBOARD_PATH: String = "clipboard_path"
internal const val KEY_CLIPBOARD_CONTENT: String = "clipboard_content"
internal const val KEY_CLIPBOARD_SOURCE_ENDPOINT_ID: String = "clipboard_source_endpoint_id"
internal const val KEY_CLIPBOARD_SOURCE_DEVICE: String = "clipboard_source_device"
internal const val KEY_CLIPBOARD_SHA256: String = "clipboard_sha256"

private fun stateBundle(
    endpointId: String = "",
    devices: List<io.github.vafu.mist.core.ManagedDevice> = emptyList(),
    startupError: String? = null,
): Bundle = Bundle().apply {
    putBoolean(KEY_READY, endpointId.isNotEmpty())
    putString(KEY_ENDPOINT_ID, endpointId)
    putString(KEY_STARTUP_ERROR, startupError)
    putParcelableArrayList(
        KEY_DEVICES,
        ArrayList(devices.map { device ->
            Bundle().apply {
                putString("endpoint_id", device.endpointId)
                putString("display_name", device.device.displayName)
                putString("device_class", device.device.deviceClass.name)
                putString("platform", device.device.platform.name)
                putString("status", device.status.name)
                putString("transport", device.transport.name)
                putBoolean("selected", device.selected)
            }
        }),
    )
}

internal fun Notification.toBundle(): Bundle = Bundle().apply {
    putString(KEY_ID, id)
    putString(KEY_APPLICATION, application)
    putString(KEY_TITLE, title)
    putString(KEY_BODY, body)
    putLong(KEY_TIMESTAMP_MS, timestampMs)
    putBoolean(KEY_SILENT, silent)
    putString(KEY_NOTIFICATION_KIND, kind.name)
    appIcon?.let { putBundle(KEY_APP_ICON, it.toBundle()) }
    image?.let { putBundle(KEY_IMAGE, it.toBundle()) }
}

/** Decodes the image file into a bitmap for the notification, if readable. */
internal fun NotificationImage.loadBitmap(): android.graphics.Bitmap? =
    runCatching { android.graphics.BitmapFactory.decodeFile(path) }.getOrNull()

internal fun NotificationImage.toBundle(): Bundle = Bundle().apply {
    putString(KEY_IMAGE_MIME_TYPE, mimeType)
    putString(KEY_IMAGE_PATH, path)
}

private fun Bundle.toNotificationImage(): NotificationImage = NotificationImage(
    mimeType = getString(KEY_IMAGE_MIME_TYPE).orEmpty(),
    path = getString(KEY_IMAGE_PATH).orEmpty(),
)

private fun Bundle.toMistNotification(): Notification = Notification(
    id = getString(KEY_ID).orEmpty(),
    application = getString(KEY_APPLICATION).orEmpty(),
    title = getString(KEY_TITLE).orEmpty(),
    body = getString(KEY_BODY).orEmpty(),
    timestampMs = getLong(KEY_TIMESTAMP_MS),
    silent = getBoolean(KEY_SILENT),
    kind = NotificationKind.valueOf(getString(KEY_NOTIFICATION_KIND).orEmpty()),
    appIcon = getBundle(KEY_APP_ICON)?.toNotificationImage(),
    image = getBundle(KEY_IMAGE)?.toNotificationImage(),
)

internal fun ClipboardContent.toBundle(): Bundle = Bundle().apply {
    when (this@toBundle) {
        is ClipboardContent.Text -> {
            putString(KEY_CLIPBOARD_KIND, "text")
            putString(KEY_CLIPBOARD_TEXT, text)
        }
        is ClipboardContent.Blob -> {
            putString(KEY_CLIPBOARD_KIND, "blob")
            putString(KEY_CLIPBOARD_MIME_TYPE, mimeType)
            putString(KEY_CLIPBOARD_PATH, path)
        }
    }
}

internal fun Bundle.toClipboardContent(): ClipboardContent =
    when (getString(KEY_CLIPBOARD_KIND)) {
        "text" -> ClipboardContent.Text(getString(KEY_CLIPBOARD_TEXT).orEmpty())
        "blob" -> ClipboardContent.Blob(
            getString(KEY_CLIPBOARD_MIME_TYPE).orEmpty(),
            getString(KEY_CLIPBOARD_PATH).orEmpty(),
        )
        else -> error("unknown clipboard content")
    }

internal fun io.github.vafu.mist.core.ReceivedClipboard.toBundle(): Bundle =
    Bundle().apply {
        putString(KEY_CLIPBOARD_SOURCE_ENDPOINT_ID, sourceEndpointId)
        putString(KEY_CLIPBOARD_SOURCE_DEVICE, sourceDevice.displayName)
        putString(KEY_CLIPBOARD_SHA256, sha256)
        putBundle(KEY_CLIPBOARD_CONTENT, content.toBundle())
    }

private fun extension(mimeType: String): String =
    when (mimeType) {
        "image/png" -> "png"
        "image/jpeg" -> "jpg"
        "image/webp" -> "webp"
        "image/gif" -> "gif"
        else -> "bin"
    }
