package io.github.vafu.mist

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning

@Composable
internal actual fun rememberQrScanner(
    onResult: (String) -> Unit,
    onError: (String) -> Unit,
): QrScanner {
    val context = LocalContext.current
    val currentOnResult = rememberUpdatedState(onResult)
    val currentOnError = rememberUpdatedState(onError)
    val scanner = remember(context) {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(context, options)
    }

    return remember(scanner) {
        object : QrScanner {
            override fun scan() {
                scanner.startScan()
                    .addOnSuccessListener { barcode ->
                        barcode.rawValue
                            ?.let(currentOnResult.value)
                            ?: currentOnError.value("The QR code has no text payload")
                    }
                    .addOnFailureListener { failure ->
                        currentOnError.value(failure.message ?: "Could not scan the QR code")
                    }
            }
        }
    }
}
