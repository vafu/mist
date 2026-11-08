package io.github.vafu.mist.android

import android.content.Intent
import android.net.Uri
import android.os.Build

/** Content handed to Mist through an Android share intent. */
public sealed interface SharePayload {
    public data class Text(val text: String) : SharePayload
    public data class Image(val uri: Uri, val mimeType: String) : SharePayload
}

public fun Intent.toSharePayload(): SharePayload? {
    if (action != Intent.ACTION_SEND) return null
    val mimeType = type.orEmpty()
    if (mimeType.startsWith("image/")) {
        val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            @Suppress("DEPRECATION")
            getParcelableExtra(Intent.EXTRA_STREAM)
        }
        if (uri != null) return SharePayload.Image(uri, mimeType)
    }
    if (mimeType.startsWith("text/") || mimeType.isEmpty()) {
        val text = getCharSequenceExtra(Intent.EXTRA_TEXT)?.toString()
        if (!text.isNullOrBlank()) return SharePayload.Text(text)
    }
    return null
}
