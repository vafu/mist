package io.github.vafu.mist.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.service.notification.StatusBarNotification
import android.util.Log
import io.github.vafu.mist.core.NotificationImage
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * Extracts notification artwork as PNG files the runtime can stream.
 *
 * Both signals are optional; missing artwork simply produces `null`.
 */
internal fun StatusBarNotification.smallIconImage(context: android.content.Context): NotificationImage? {
    val icon = notification.smallIcon ?: return null
    val drawable = runCatching { icon.loadDrawable(context) }.getOrNull() ?: return null
    return drawable.toPng(context, "icon")
}

internal fun StatusBarNotification.largeImage(context: android.content.Context): NotificationImage? {
    val extras = notification.extras ?: return null
    val bitmap = runCatching {
        extras.bitmap(android.app.Notification.EXTRA_PICTURE)
            ?: extras.bitmap(android.app.Notification.EXTRA_LARGE_ICON)
    }.getOrNull() ?: return null
    return bitmap.toPng(context, "image")
}

@Suppress("DEPRECATION")
private fun android.os.Bundle.bitmap(key: String): Bitmap? {
    @Suppress("UNCHECKED_CAST")
    return getParcelable(key) as? Bitmap
}

private fun Drawable.toPng(context: android.content.Context, kind: String): NotificationImage? =
    runCatching {
        val bitmap = if (this is BitmapDrawable && bitmap != null) {
            bitmap
        } else {
            val width = intrinsicWidth.coerceAtLeast(1)
            val height = intrinsicHeight.coerceAtLeast(1)
            Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).also { canvas ->
                setBounds(0, 0, width, height)
                draw(Canvas(canvas))
            }
        }
        bitmap.toPng(context, kind)
    }.onFailure { Log.w(TAG, "could not encode $kind", it) }.getOrNull()

private fun Bitmap.toPng(context: android.content.Context, kind: String): NotificationImage? =
    runCatching {
        val directory = File(context.cacheDir, "notification-images").apply { mkdirs() }
        val file = File(directory, "$kind-${System.currentTimeMillis()}.png")
        ByteArrayOutputStream().use { output ->
            compress(Bitmap.CompressFormat.PNG, 100, output)
            file.writeBytes(output.toByteArray())
        }
        NotificationImage("image/png", file.absolutePath)
    }.onFailure { Log.w(TAG, "could not write $kind", it) }.getOrNull()

private const val TAG = "NotificationImages"
