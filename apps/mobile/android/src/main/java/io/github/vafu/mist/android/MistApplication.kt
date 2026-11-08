package io.github.vafu.mist.android

import android.app.Application
import android.os.Build
import java.io.File

/** Starts the long-lived runtime service when the main Mist process is created. */
public class MistApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        if (isMainProcess()) {
            MistServiceStarter.start(this)
        }
    }

    private fun isMainProcess(): Boolean {
        val processName = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            getProcessName()
        } else {
            runCatching { File("/proc/self/cmdline").readText().trimEnd('\u0000') }
                .getOrDefault("")
        }
        return processName == packageName
    }
}
