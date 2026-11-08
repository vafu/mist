package io.github.vafu.mist.android

import android.app.Notification
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.util.Log
import io.github.vafu.mist.core.Notification as MistNotification
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

public class MistNotificationListenerService : NotificationListenerService() {
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val runtimeService = CompletableDeferred<IMistService>()
    private val connection: ServiceConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            runtimeService.complete(IMistService.Stub.asInterface(binder))
        }

        override fun onServiceDisconnected(name: ComponentName) = Unit
    }

    override fun onCreate() {
        super.onCreate()
        bindService(
            Intent(this, MistRuntimeService::class.java).setAction(MistServiceStarter.ACTION_START),
            connection,
            Context.BIND_AUTO_CREATE,
        )
    }

    override fun onNotificationPosted(status: StatusBarNotification) {
        if (status.packageName == packageName ||
            status.notification.flags and Notification.FLAG_GROUP_SUMMARY != 0
        ) {
            return
        }
        val extras = status.notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val body = extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString()
            ?: extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
        if (title.isBlank() && body.isBlank()) {
            return
        }
        val applicationName = runCatching {
            val info = packageManager.getApplicationInfo(status.packageName, 0)
            packageManager.getApplicationLabel(info).toString()
        }.getOrDefault(status.packageName)

        scope.launch {
            runCatching {
                runtimeService.await().pushNotification(
                    MistNotification(
                        id = "${status.packageName}:${status.tag.orEmpty()}:${status.id}",
                        application = applicationName,
                        title = title,
                        body = body,
                        timestampMs = status.postTime,
                        silent = false,
                        kind = status.notificationKind(),
                    ).toBundle(),
                )
            }.onFailure { error ->
                Log.e(TAG, "Could not forward Android notification through Mist", error)
            }
        }
    }

    override fun onDestroy() {
        runCatching { unbindService(connection) }
        scope.cancel()
        super.onDestroy()
    }

    private companion object {
        const val TAG: String = "MistNotificationListener"
    }
}
