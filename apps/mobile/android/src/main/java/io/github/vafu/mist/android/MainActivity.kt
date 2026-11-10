package io.github.vafu.mist.android

import android.Manifest
import android.app.NotificationManager
import android.content.ComponentName
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import io.github.vafu.mist.MistApp
import kotlinx.coroutines.launch

public class MainActivity : ComponentActivity() {
    private var notificationPostingGranted: Boolean by mutableStateOf(false)
    private var notificationListenerGranted: Boolean by mutableStateOf(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        updateNotificationPermissions()
        setContent {
            val controller = remember { AndroidMistController(this) }
            val serviceState by controller.state.collectAsState()
            DisposableEffect(controller) {
                onDispose(controller::close)
            }
            val notificationPermission = rememberLauncherForActivityResult(
                ActivityResultContracts.RequestPermission(),
            ) { updateNotificationPermissions() }
            val scope = rememberCoroutineScope()
            val shareFilePicker = rememberLauncherForActivityResult(
                ActivityResultContracts.OpenDocument(),
            ) { uri ->
                if (uri != null) {
                    scope.launch {
                        val path = controller.cacheSharedFile(uri, displayName(uri))
                        if (path != null) {
                            runCatching { controller.shareFile(path) }
                        }
                    }
                }
            }

            MaterialTheme {
                Surface {
                    when {
                        serviceState.startupError != null -> Text(serviceState.startupError.orEmpty())
                        !serviceState.ready -> CircularProgressIndicator()
                        else -> MistApp(
                            controller = controller,
                            notificationPostingGranted = notificationPostingGranted,
                            notificationListenerGranted = notificationListenerGranted,
                            requestNotificationPosting = {
                                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                                    notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
                                }
                            },
                            requestNotificationListener = {
                                startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS))
                            },
                            pickAndShareFile = { shareFilePicker.launch(arrayOf("*/*")) },
                        )
                    }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        updateNotificationPermissions()
    }

    private fun updateNotificationPermissions() {
        notificationPostingGranted = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
        val component = ComponentName(this, MistNotificationListenerService::class.java)
        notificationListenerGranted = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            getSystemService(NotificationManager::class.java)
                .isNotificationListenerAccessGranted(component)
        } else {
            Settings.Secure.getString(contentResolver, "enabled_notification_listeners")
                ?.split(':')
                ?.mapNotNull(ComponentName::unflattenFromString)
                ?.contains(component) == true
        }
    }

    private fun displayName(uri: android.net.Uri): String? =
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
}
