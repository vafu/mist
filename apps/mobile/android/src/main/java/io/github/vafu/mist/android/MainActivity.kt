package io.github.vafu.mist.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import io.github.vafu.mist.MistApp

public class MainActivity : ComponentActivity() {
    private val model: MistViewModel by viewModels()
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MistApp(model.runtime, model.error)
        }
    }
}
