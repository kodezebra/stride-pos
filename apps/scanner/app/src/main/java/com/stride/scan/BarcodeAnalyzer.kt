package com.stride.scan

import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage

enum class Mode { NONE, PAIRING, SCANNING }

/**
 * Reads the camera and hands the first usable code to the activity.
 *
 * The frame is closed only when ML Kit is done with it; closing it earlier is
 * the classic way to make the preview freeze. Pairing mode accepts QR codes,
 * scanning mode accepts product barcodes. Nothing here talks to the till.
 */
class BarcodeAnalyzer(
    private val scanner: BarcodeScanner,
    private val mode: () -> Mode,
    private val onPairingCode: (String) -> Unit,
    private val onBarcode: (String) -> Unit,
) : ImageAnalysis.Analyzer {

    @ExperimentalGetImage
    override fun analyze(proxy: ImageProxy) {
        val current = mode()
        val media = proxy.image
        if (current == Mode.NONE || media == null) {
            proxy.close()
            return
        }

        val image = InputImage.fromMediaImage(media, proxy.imageInfo.rotationDegrees)
        scanner.process(image)
            .addOnSuccessListener { codes ->
                val match = if (current == Mode.PAIRING) {
                    codes.firstOrNull {
                        it.format == Barcode.FORMAT_QR_CODE && !it.rawValue.isNullOrBlank()
                    }
                } else {
                    codes.firstOrNull {
                        it.format != Barcode.FORMAT_QR_CODE && !it.rawValue.isNullOrBlank()
                    }
                }
                val raw = match?.rawValue ?: return@addOnSuccessListener
                if (current == Mode.PAIRING) onPairingCode(raw) else onBarcode(raw)
            }
            .addOnCompleteListener { proxy.close() }
    }
}
