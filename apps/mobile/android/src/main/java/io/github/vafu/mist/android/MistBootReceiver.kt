package io.github.vafu.mist.android

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** Restarts the Mist runtime after the device boots. */
public class MistBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) {
            MistServiceStarter.start(context)
        }
    }
}
