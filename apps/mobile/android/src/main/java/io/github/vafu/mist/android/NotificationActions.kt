package io.github.vafu.mist.android

import android.app.Notification
import android.service.notification.StatusBarNotification
import io.github.vafu.mist.core.NotificationAction

/**
 * Extracts actionable buttons from a notification.
 *
 * Android action titles are the user-visible labels; the synthetic key is derived
 * from a stable hash of the title because `Notification.Action` exposes no id.
 * Mist assigns sequential ids instead, which are stable for the lifetime of the
 * forwarded notification.
 */
internal fun StatusBarNotification.notificationActions(): List<NotificationAction> {
    val posted = notification
    if (posted.actions == null) return emptyList()
    return posted.actions
        .mapIndexedNotNull { index, action ->
            val label = action.title?.toString().orEmpty()
            if (label.isBlank()) return@mapIndexedNotNull null
            NotificationAction(id = "a$index", label = label)
        }
        .take(MAX_ACTIONS)
}

private const val MAX_ACTIONS = 3
