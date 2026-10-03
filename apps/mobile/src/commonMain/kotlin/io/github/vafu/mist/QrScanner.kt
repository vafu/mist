package io.github.vafu.mist

import androidx.compose.runtime.Composable

/** Platform entry point for scanning a pairing offer from a QR code. */
internal interface QrScanner {
    fun scan()
}

@Composable
internal expect fun rememberQrScanner(
    onResult: (String) -> Unit,
    onError: (String) -> Unit,
): QrScanner
