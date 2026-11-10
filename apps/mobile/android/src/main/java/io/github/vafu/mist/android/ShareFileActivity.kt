package io.github.vafu.mist.android

import android.app.Activity
import android.os.Bundle

/** Share-sheet entry point that offers the shared document as a file transfer. */
public class ShareFileActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        intent.toSharePayload()?.let { payload ->
            startService(MistServiceStarter.shareIntent(this, payload, asFile = true))
        }
        finish()
    }
}
