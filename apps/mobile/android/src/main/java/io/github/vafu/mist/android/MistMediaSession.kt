package io.github.vafu.mist.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.media.MediaMetadata as PlatformMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import io.github.vafu.mist.core.MediaCommand
import io.github.vafu.mist.core.MediaPlayer
import io.github.vafu.mist.core.MediaOrigin
import io.github.vafu.mist.core.PlaybackStatus

/**
 * Exposes remote media players as a local [MediaSession].
 *
 * This is what makes a player running on another Mist device controllable from
 * Android's media notification, lock screen, and Bluetooth controls. The
 * session carries no playback of its own: every transport action is forwarded
 * to the remote player through the shared runtime.
 */
public class MistMediaSession(
    private val service: MistRuntimeService,
    private val send: (String, MediaCommand) -> Unit,
) {
    private val session = MediaSession(service, "Mist")
    private val mainHandler = android.os.Handler(android.os.Looper.getMainLooper())
    private var current: MediaPlayer? = null

    private val callback = object : MediaSession.Callback() {
        override fun onPlay() = dispatch(MediaCommand.PLAY)
        override fun onPause() = dispatch(MediaCommand.PAUSE)
        override fun onStop() = dispatch(MediaCommand.STOP)
        override fun onSkipToNext() = dispatch(MediaCommand.NEXT)
        override fun onSkipToPrevious() = dispatch(MediaCommand.PREVIOUS)

        override fun onPlayFromMediaId(mediaId: String?, extras: android.os.Bundle?) =
            dispatch(MediaCommand.PLAY)
    }

    init {
        createChannel()
    }

    /**
     * Creates the underlying session on the main thread.
     *
     * `MediaSession` builds its callback handler with the calling thread's
     * `Looper`, so construction must not happen on a background coroutine.
     */
    public fun start() {
        mainHandler.post {
            session.setCallback(callback, mainHandler)
            session.setMediaButtonReceiver(
                PendingIntent.getBroadcast(
                    service,
                    0,
                    Intent(Intent.ACTION_MEDIA_BUTTON).setClass(
                        service,
                        MistMediaButtonReceiver::class.java,
                    ),
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
        }
    }

    /** Updates the session to reflect the current remote players. */
    public fun update(players: List<MediaPlayer>) {
        val remote = players.filter { it.origin == MediaOrigin.REMOTE }
        // Prefer a playing player, otherwise surface the first known one.
        val next = remote.firstOrNull { it.status == PlaybackStatus.PLAYING } ?: remote.firstOrNull()
        if (next == null) {
            current = null
            mainHandler.post { session.isActive = false }
            cancelNotification()
            return
        }
        current = next
        mainHandler.post {
            session.setMetadata(next.toPlatformMetadata())
            session.setPlaybackState(next.toPlaybackState())
            session.isActive = true
        }
        postNotification(next)
    }

    /** Remote player currently represented by this session, if any. */
    public fun remotePlayerId(): String? = current?.id

    /** Releases the session and its notification. */
    public fun release() {
        session.isActive = false
        cancelNotification()
        session.release()
    }

    private fun dispatch(command: MediaCommand) {
        val player = current ?: return
        send(player.id, command)
    }

    private fun createChannel() {
        val manager = service.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Mist media controls",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
    }

    private fun postNotification(player: MediaPlayer) {
        val notification = Notification.Builder(service, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(player.metadata.title.ifBlank { player.name })
            .setContentText(player.metadata.artist)
            .setSubText("Mist · ${player.device.displayName}")
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setOngoing(player.status == PlaybackStatus.PLAYING)
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(session.sessionToken)
                    .setShowActionsInCompactView(0, 1, 2),
            )
            .addAction(
                Notification.Action.Builder(
                    android.R.drawable.ic_media_previous,
                    "Previous",
                    actionIntent(MediaCommand.PREVIOUS, 1),
                ).build(),
            )
            .addAction(
                Notification.Action.Builder(
                    if (player.status == PlaybackStatus.PLAYING) {
                        android.R.drawable.ic_media_pause
                    } else {
                        android.R.drawable.ic_media_play
                    },
                    if (player.status == PlaybackStatus.PLAYING) "Pause" else "Play",
                    actionIntent(
                        if (player.status == PlaybackStatus.PLAYING) {
                            MediaCommand.PAUSE
                        } else {
                            MediaCommand.PLAY
                        },
                        2,
                    ),
                ).build(),
            )
            .addAction(
                Notification.Action.Builder(
                    android.R.drawable.ic_media_next,
                    "Next",
                    actionIntent(MediaCommand.NEXT, 3),
                ).build(),
            )
            .build()
        runCatching {
            service.getSystemService(NotificationManager::class.java)
                .notify(NOTIFICATION_ID, notification)
        }
    }

    private fun cancelNotification() {
        runCatching {
            service.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
        }
    }

    private fun actionIntent(command: MediaCommand, requestCode: Int): PendingIntent {
        val intent = Intent(service, MistRuntimeService::class.java).apply {
            action = MistServiceStarter.ACTION_MEDIA_COMMAND
            putExtra(MistServiceStarter.EXTRA_MEDIA_COMMAND, command.name)
        }
        val flags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        } else {
            PendingIntent.FLAG_UPDATE_CURRENT
        }
        return PendingIntent.getService(service, requestCode, intent, flags)
    }

    private companion object {
        const val CHANNEL_ID = "mist_media"
        const val NOTIFICATION_ID = 2
    }
}

private fun MediaPlayer.toPlatformMetadata(): PlatformMetadata =
    PlatformMetadata.Builder()
        .putString(PlatformMetadata.METADATA_KEY_TITLE, metadata.title.ifBlank { name })
        .putString(PlatformMetadata.METADATA_KEY_ARTIST, metadata.artist)
        .putString(PlatformMetadata.METADATA_KEY_ALBUM, metadata.album)
        .apply {
            metadata.durationMs?.let {
                putLong(PlatformMetadata.METADATA_KEY_DURATION, it.toLong())
            }
        }
        .build()

private fun MediaPlayer.toPlaybackState(): PlaybackState {
    var actions = PlaybackState.ACTION_PLAY_PAUSE or
        PlaybackState.ACTION_STOP or
        PlaybackState.ACTION_PLAY or
        PlaybackState.ACTION_PAUSE or
        PlaybackState.ACTION_SKIP_TO_NEXT or
        PlaybackState.ACTION_SKIP_TO_PREVIOUS
    val state = when (status) {
        PlaybackStatus.PLAYING -> PlaybackState.STATE_PLAYING
        PlaybackStatus.PAUSED -> PlaybackState.STATE_PAUSED
        PlaybackStatus.STOPPED -> PlaybackState.STATE_STOPPED
    }
    return PlaybackState.Builder()
        .setActions(actions)
        .setState(state, positionMs?.toLong() ?: 0L, 1.0f)
        .build()
}
