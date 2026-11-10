package io.github.vafu.mist.android

import android.media.MediaDescription
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import android.media.session.PlaybackState
import io.github.vafu.mist.core.MediaCapabilities
import io.github.vafu.mist.core.MediaArtwork
import io.github.vafu.mist.core.MediaCommand
import io.github.vafu.mist.core.MediaMetadata
import io.github.vafu.mist.core.PlaybackStatus
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlin.coroutines.resume

/**
 * Android media session access in the `:mist` process.
 *
 * Discovery uses [MediaSessionManager] with the notification-listener component
 * as the permitted controller, the only no-privilege way to observe other apps'
 * playback state. Commands go through each session's [MediaController].
 */
public class AndroidMedia(private val service: MistRuntimeService) {
    private val manager: MediaSessionManager? =
        service.getSystemService(MediaSessionManager::class.java)

    private val handler = android.os.Handler(android.os.Looper.getMainLooper())
    private val sessionListener =
        MediaSessionManager.OnActiveSessionsChangedListener { controllers -> publish(controllers) }

    private val component = android.content.ComponentName(
        service,
        MistNotificationListenerService::class.java,
    )

    private val callbacks = mutableMapOf<String, MediaController.Callback>()

    public fun start() {
        val manager = manager ?: run {
            android.util.Log.e("AndroidMedia", "no MediaSessionManager")
            return
        }
        runCatching {
            manager.addOnActiveSessionsChangedListener(sessionListener, component, handler)
        }
        runCatching { publish(manager.getActiveSessions(component)) }
            .onFailure { android.util.Log.e("AndroidMedia", "getActiveSessions failed", it) }
    }

    public fun stop() {
        runCatching {
            manager?.removeOnActiveSessionsChangedListener(sessionListener)
        }
        callbacks.clear()
    }

    private fun publish(controllers: List<MediaController>?) {
        // Skip our own exported session; re-importing it would publish a remote
        // player back to peers as if it were local.
        val players = controllers
            .orEmpty()
            .filter { it.packageName != service.packageName }
            .mapNotNull(::snapshot)
        service.onLocalMediaChanged(players)
        registerCallbacks(controllers.orEmpty())
    }

    private fun registerCallbacks(controllers: List<MediaController>) {
        val active = controllers.mapNotNull { it.sessionToken?.toString() }.toSet()
        callbacks.keys.retainAll(active)
        for (controller in controllers) {
            val token = controller.sessionToken?.toString() ?: continue
            if (callbacks.containsKey(token)) continue
            val callback = object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: PlaybackState?) = republish()
                override fun onMetadataChanged(metadata: android.media.MediaMetadata?) = republish()
                override fun onSessionDestroyed() = republish()
            }
            runCatching { controller.registerCallback(callback, android.os.Handler(android.os.Looper.getMainLooper())) }
            callbacks[token] = callback
        }
    }

    private fun republish() {
        val manager = manager ?: return
        runCatching { publish(manager.getActiveSessions(component)) }
            .onFailure { android.util.Log.e("AndroidMedia", "getActiveSessions failed", it) }
    }

    private fun snapshot(controller: MediaController): LocalMediaPlayer? {
        val playback = controller.playbackState ?: return null
        val metadata = controller.metadata
        val description: MediaDescription? = metadata?.description
        val actions = playback.actions
        val commands = buildList {
            if (actions and PlaybackState.ACTION_PLAY != 0L) add(MediaCommand.PLAY)
            if (actions and PlaybackState.ACTION_PAUSE != 0L) add(MediaCommand.PAUSE)
            if (actions and PlaybackState.ACTION_PLAY_PAUSE != 0L) add(MediaCommand.PLAY_PAUSE)
            if (actions and PlaybackState.ACTION_SKIP_TO_NEXT != 0L) add(MediaCommand.NEXT)
            if (actions and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L) add(MediaCommand.PREVIOUS)
            if (actions and PlaybackState.ACTION_STOP != 0L) add(MediaCommand.STOP)
        }
        return LocalMediaPlayer(
            id = controller.sessionToken?.toString() ?: controller.packageName,
            name = controller.packageName,
            status = when (playback.state) {
                PlaybackState.STATE_PLAYING -> PlaybackStatus.PLAYING
                PlaybackState.STATE_PAUSED,
                PlaybackState.STATE_BUFFERING,
                -> PlaybackStatus.PAUSED
                else -> PlaybackStatus.STOPPED
            },
            metadata = MediaMetadata(
                title = description?.title?.toString()
                    ?: metadata?.getString(android.media.MediaMetadata.METADATA_KEY_TITLE)
                    ?: "",
                artist = description?.subtitle?.toString()
                    ?: metadata?.getString(android.media.MediaMetadata.METADATA_KEY_ARTIST)
                    ?: "",
                album = metadata?.getString(android.media.MediaMetadata.METADATA_KEY_ALBUM) ?: "",
                artUrl = "",
                artwork = artwork(metadata, controller),
                durationMs = metadata
                    ?.getLong(android.media.MediaMetadata.METADATA_KEY_DURATION)
                    ?.takeIf { it > 0 }
                    ?.toULong(),
            ),
            capabilities = MediaCapabilities(commands),
            positionMs = playback.position.takeIf { it >= 0 }?.toULong(),
            token = controller.sessionToken?.toString() ?: controller.packageName,
        )
    }

    /**
     * Writes the session's artwork bitmap to a private cache file.
     *
     * MediaSession exposes artwork as a Bitmap, not bytes; the runtime needs a
     * file path so it can import and share it as a blob.
     */
    private fun artwork(
        metadata: android.media.MediaMetadata?,
        controller: MediaController,
    ): MediaArtwork? {
        val bitmap = metadata?.getBitmap(android.media.MediaMetadata.METADATA_KEY_ALBUM_ART)
            ?: metadata?.getBitmap(android.media.MediaMetadata.METADATA_KEY_ART)
            ?: metadata?.getBitmap(android.media.MediaMetadata.METADATA_KEY_DISPLAY_ICON)
            ?: return null
        val directory = java.io.File(service.cacheDir, "media-artwork").apply { mkdirs() }
        val target = java.io.File(
            directory,
            "art-${(controller.sessionToken?.toString() ?: controller.packageName).hashCode()}.png",
        )
        val written = runCatching {
            target.outputStream().use { output ->
                bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output)
            }
        }.getOrDefault(false)
        if (!written) return null
        return MediaArtwork(mimeType = "image/png", path = target.absolutePath)
    }

    public suspend fun command(playerId: String, command: MediaCommand) {
        val controller = controllerFor(playerId) ?: return
        val transport = controller.transportControls
        withContext(Dispatchers.Main) {
            when (command) {
                MediaCommand.PLAY -> transport.play()
                MediaCommand.PAUSE -> transport.pause()
                MediaCommand.PLAY_PAUSE -> transport.play()
                MediaCommand.NEXT -> transport.skipToNext()
                MediaCommand.PREVIOUS -> transport.skipToPrevious()
                MediaCommand.STOP -> transport.stop()
            }
        }
    }

    private suspend fun controllerFor(playerId: String): MediaController? {
        val manager = manager ?: return null
        return withContext(Dispatchers.Main) {
            runCatching {
                manager.getActiveSessions(component).firstOrNull {
                    (it.sessionToken?.toString() ?: it.packageName) == playerId
                }
            }.getOrNull()
        }
    }
}
