package io.github.vafu.mist.android

import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import io.github.vafu.mist.MistApp
import io.github.vafu.mist.core.DeviceClass
import io.github.vafu.mist.core.DeviceInfo
import io.github.vafu.mist.core.Platform

public class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MistApp(
                DeviceInfo(
                    displayName = Build.MODEL,
                    deviceClass = DeviceClass.PHONE,
                    platform = Platform.ANDROID,
                ),
            )
        }
    }
}
