package io.github.vafu.mist.android

import io.github.vafu.mist.core.MediaCapabilities
import io.github.vafu.mist.core.MediaMetadata
import io.github.vafu.mist.core.PlaybackStatus

/** Platform player snapshot passed to the Rust runtime. */
internal data class LocalMediaPlayer(
    val id: String,
    val name: String,
    val status: PlaybackStatus,
    val metadata: MediaMetadata,
    val capabilities: MediaCapabilities,
    val positionMs: ULong?,
    val token: String,
)
