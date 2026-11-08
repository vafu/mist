package io.github.vafu.mist.android

import android.app.Notification
import android.service.notification.StatusBarNotification
import io.github.vafu.mist.core.NotificationKind

/**
 * Classifies a posted notification for the shared runtime.
 *
 * The runtime decides which feature consumes each kind; media transport updates
 * are dropped by the generic notification bridge and will be handled by the
 * media feature. Classification requires platform knowledge, so it happens here.
 *
 * The public signals are:
 * - `EXTRA_MEDIA_SESSION`, attached to MediaSession transport notifications;
 * - `CATEGORY_TRANSPORT`, the documented category for media transport.
 *
 * The `MediaStyle` template itself is a hidden API and cannot be inspected.
 */
internal fun StatusBarNotification.notificationKind(): NotificationKind {
    val posted = notification
    if (posted.extras?.containsKey(Notification.EXTRA_MEDIA_SESSION) == true ||
        posted.category == Notification.CATEGORY_TRANSPORT
    ) {
        return NotificationKind.MEDIA
    }
    return NotificationKind.MESSAGE
}
