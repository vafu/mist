package io.github.vafu.mist.android

import android.app.Activity
import android.os.Bundle

/**
 * Share-sheet entry point for "Send to clipboard".
 *
 * It carries no behavior of its own: it forwards the share payload to the
 * background runtime service, which owns all sending, and finishes immediately.
 */
public class ShareToClipboardActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        intent.toSharePayload()?.let { payload ->
            startService(MistServiceStarter.shareIntent(this, payload))
        }
        finish()
    }
}
