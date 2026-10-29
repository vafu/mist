package io.github.vafu.mist.android

import android.app.Application
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.vafu.mist.core.DeviceInfo
import io.github.vafu.mist.core.MistRuntime
import io.github.vafu.mist.core.Platform
import io.github.vafu.mist.core.createMistRuntime
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Retains the Rust instance across configuration changes; owns no domain state. */
internal class MistViewModel(application: Application) : AndroidViewModel(application) {
    var runtime: MistRuntime? by mutableStateOf(null)
        private set
    var error: String? by mutableStateOf(null)
        private set

    init {
        viewModelScope.launch {
            val current = try {
                createMistRuntime(
                    DeviceInfo(displayName = Build.MODEL, platform = Platform.ANDROID),
                    application.filesDir.resolve("mist").absolutePath,
                )
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                error = failure.message ?: "Mist runtime failed"
                return@launch
            }
            try {
                runtime = current
                awaitCancellation()
            } finally {
                runtime = null
                withContext(NonCancellable) {
                    try {
                        current.shutdown()
                    } finally {
                        current.close()
                    }
                }
            }
        }
    }
}
