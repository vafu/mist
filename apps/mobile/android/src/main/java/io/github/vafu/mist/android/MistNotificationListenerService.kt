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
    /** Notifications forwarded to peers, keyed by the id Mist assigned. */
    private val forwarded: MutableSet<String> = java.util.Collections.synchronizedSet(mutableSetOf())
    /** Maps the id Mist assigned back to the Android notification key. */
    private val keys: MutableMap<String, String> =
        java.util.Collections.synchronizedMap(mutableMapOf())
    private val runtimeService = CompletableDeferred<IMistService>()
    private val connection: ServiceConnection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val service = IMistService.Stub.asInterface(binder)
            runtimeService.complete(service)
            runCatching {
                service.registerCallback(object : IMistCallback.Stub() {
                    override fun onStateChanged(state: android.os.Bundle) = Unit

                    override fun onClipboardReceived(content: android.os.Bundle) = Unit

                    override fun onNotificationAction(activation: android.os.Bundle) = Unit

                    override fun onNotificationConsumed(consumption: android.os.Bundle) {
                        val id = consumption.getString(KEY_ACTIVATION_NOTIFICATION) ?: return
                        withdraw(id)
                    }
                })
            }
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

        val notificationId = "${status.packageName}:${status.tag.orEmpty()}:${status.id}"
        forwarded.add(notificationId)
        keys[notificationId] = status.key
        scope.launch {
            runCatching {
                runtimeService.await().pushNotification(
                    MistNotification(
                        id = notificationId,
                        application = applicationName,
                        title = title,
                        body = body,
                        timestampMs = status.postTime,
                        silent = false,
                        kind = status.notificationKind(),
                        appIcon = status.smallIconImage(this@MistNotificationListenerService),
                        image = status.largeImage(this@MistNotificationListenerService),
                        actions = status.notificationActions(),
                    ).toBundle(),
                )
            }.onFailure { error ->
                Log.e(TAG, "Could not forward Android notification through Mist", error)
            }
        }
    }

    /** Cancels the local notification once a peer consumed it. */
    private fun withdraw(notificationId: String) {
        forwarded.remove(notificationId)
        val key = keys.remove(notificationId) ?: return
        runCatching { cancelNotification(key) }
            .onFailure { Log.w(TAG, "Could not withdraw a consumed notification", it) }
    }

    override fun onNotificationRemoved(status: StatusBarNotification) {
        val notificationId = "${status.packageName}:${status.tag.orEmpty()}:${status.id}"
        if (!forwarded.remove(notificationId)) {
            return
        }
        scope.launch {
            runCatching {
                val service = runtimeService.await()
                // Undelivered copies are dropped, delivered ones removed remotely.
                service.retractNotification(notificationId, noResult)
                service.discardNotification(notificationId)
            }.onFailure { error ->
                Log.e(TAG, "Could not report discarded Android notification", error)
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
        const val KEY_ACTIVATION_NOTIFICATION: String = "activation_notification"

        /** Listener callbacks are fire-and-forget; failures are logged only. */
        val noResult: IMistResultCallback = object : IMistResultCallback.Stub() {
            override fun onSuccess(result: android.os.Bundle) = Unit

            override fun onFailure(message: String) {
                Log.w(TAG, "Mist runtime rejected a notification event: $message")
            }
        }
    }
}
