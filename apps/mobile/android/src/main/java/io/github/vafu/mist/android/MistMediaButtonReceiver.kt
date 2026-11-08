package io.github.vafu.mist.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.view.KeyEvent

/**
 * Routes hardware and lockscreen media buttons to the Mist runtime service.
 *
 * `MediaSession` alone does not receive key events from the media button
 * route; a receiver must forward them to the service, which dispatches the
 * matching transport command to the remote player.
 */
public class MistMediaButtonReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_MEDIA_BUTTON) return
        val event = intent.getParcelableExtra<KeyEvent>(Intent.EXTRA_KEY_EVENT) ?: return
        if (event.action != KeyEvent.ACTION_DOWN) return
        val command = when (event.keyCode) {
            KeyEvent.KEYCODE_MEDIA_PLAY -> "PLAY"
            KeyEvent.KEYCODE_MEDIA_PAUSE -> "PAUSE"
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE -> "PLAY_PAUSE"
            KeyEvent.KEYCODE_MEDIA_NEXT -> "NEXT"
            KeyEvent.KEYCODE_MEDIA_PREVIOUS -> "PREVIOUS"
            KeyEvent.KEYCODE_MEDIA_STOP -> "STOP"
            else -> return
        }
        context.startService(
            MistServiceStarter.mediaCommandIntent(context, command),
        )
    }
}
