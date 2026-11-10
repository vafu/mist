package io.github.vafu.mist.android

import android.content.Context
import android.content.Intent
import android.util.Log

/** Process-independent control surface for the Mist background runtime service. */
internal object MistServiceStarter {
    const val ACTION_START: String = "io.github.vafu.mist.action.START"
    const val ACTION_STOP: String = "io.github.vafu.mist.action.STOP"
    const val ACTION_SHARE: String = "io.github.vafu.mist.action.SHARE"
    const val ACTION_MEDIA_COMMAND: String = "io.github.vafu.mist.action.MEDIA_COMMAND"
    const val EXTRA_MEDIA_COMMAND: String = "media_command"
    const val ACTION_NOTIFICATION_ACTION: String = "io.github.vafu.mist.action.NOTIFICATION_ACTION"
    const val EXTRA_NOTIFICATION_ENDPOINT: String = "notification_endpoint"
    const val EXTRA_NOTIFICATION_ID: String = "notification_id"
    const val EXTRA_NOTIFICATION_ACTION: String = "notification_action"
    private const val EXTRA_SHARE_TEXT: String = "share_text"
    private const val EXTRA_SHARE_URI: String = "share_uri"
    private const val EXTRA_SHARE_MIME_TYPE: String = "share_mime_type"
    private const val EXTRA_SHARE_AS_FILE: String = "share_as_file"

    fun serviceIntent(context: Context, action: String = ACTION_START): Intent =
        Intent(context, MistRuntimeService::class.java).setAction(action)

    fun start(context: Context) {
        runCatching {
            context.startForegroundService(serviceIntent(context, ACTION_START))
        }.onFailure { error ->
            Log.e(TAG, "Could not start the Mist runtime service", error)
        }
    }

    fun shareIntent(context: Context, payload: SharePayload, asFile: Boolean): Intent =
        serviceIntent(context, ACTION_SHARE).apply {
            putExtra(EXTRA_SHARE_AS_FILE, asFile)
            when (payload) {
                is SharePayload.Text -> putExtra(EXTRA_SHARE_TEXT, payload.text)
                is SharePayload.Image -> {
                    putExtra(EXTRA_SHARE_URI, payload.uri.toString())
                    putExtra(EXTRA_SHARE_MIME_TYPE, payload.mimeType)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                is SharePayload.File -> {
                    putExtra(EXTRA_SHARE_URI, payload.uri.toString())
                    putExtra(EXTRA_SHARE_MIME_TYPE, payload.mimeType)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
            }
        }

    /** Whether this share should be sent as a file rather than clipboard content. */
    fun shareAsFile(intent: Intent): Boolean =
        intent.getBooleanExtra(EXTRA_SHARE_AS_FILE, false)

    fun sharePayload(intent: Intent): SharePayload? {
        if (intent.action != ACTION_SHARE) return null
        intent.getStringExtra(EXTRA_SHARE_TEXT)?.let { text ->
            if (text.isNotBlank()) return SharePayload.Text(text)
        }
        val uri = intent.getStringExtra(EXTRA_SHARE_URI) ?: return null
        val mimeType = intent.getStringExtra(EXTRA_SHARE_MIME_TYPE) ?: return null
        return if (mimeType.startsWith("image/")) {
            SharePayload.Image(android.net.Uri.parse(uri), mimeType)
        } else {
            SharePayload.File(android.net.Uri.parse(uri), mimeType)
        }
    }

    fun mediaCommandIntent(context: Context, command: String): Intent =
        serviceIntent(context, ACTION_MEDIA_COMMAND).apply {
            putExtra(EXTRA_MEDIA_COMMAND, command)
        }

    fun stop(context: Context) {
        context.stopService(Intent(context, MistRuntimeService::class.java))
    }

    private const val TAG: String = "MistServiceStarter"
}
