package io.github.vafu.mist.android

import android.app.Activity
import android.os.Bundle

/** Share-sheet entry point that sends text or images as clipboard content. */
public class ShareToClipboardActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        intent.toSharePayload()?.let { payload ->
            startService(MistServiceStarter.shareIntent(this, payload, asFile = false))
        }
        finish()
    }
}
