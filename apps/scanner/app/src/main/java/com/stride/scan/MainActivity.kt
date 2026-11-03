package com.stride.scan

import android.Manifest
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.view.View
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * One screen, two jobs: pair the phone with a till, then scan for it.
 *
 * There is no account and no cloud session. The only state that outlives the
 * activity is [Pairing] in private preferences and the Sell/Add choice.
 */
class MainActivity : ComponentActivity() {

    private lateinit var prefs: SharedPreferences
    private lateinit var scanner: BarcodeScanner

    private lateinit var previewView: PreviewView
    private lateinit var topBar: View
    private lateinit var endpointText: TextView
    private lateinit var statusText: TextView
    private lateinit var scanControls: View
    private lateinit var modeGroup: RadioGroup
    private lateinit var modeSell: RadioButton
    private lateinit var modeAdd: RadioButton

    private lateinit var setupPanel: ScrollView
    private lateinit var hostInput: EditText
    private lateinit var portInput: EditText
    private lateinit var tokenInput: EditText
    private lateinit var pairStatus: TextView
    private lateinit var scanQrButton: Button
    private lateinit var connectButton: Button
    private lateinit var cancelPairButton: Button
    private lateinit var pairingHint: View
    private lateinit var pairingHintText: TextView

    private val cameraExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    private var provider: ProcessCameraProvider? = null
    private var client: TillClient? = null

    private var screen = Screen.PAIR

    @Volatile
    private var cameraMode = Mode.NONE

    private var askingForCamera = false
    private var pendingCameraUse: (() -> Unit)? = null

    private var connectAttempt = 0
    private var lastBarcode = ""
    private var lastScanAt = 0L

    private val analyzer: BarcodeAnalyzer by lazy {
        BarcodeAnalyzer(
            scanner = scanner,
            mode = { cameraMode },
            onPairingCode = { raw -> runOnUiThread { handlePairingCode(raw) } },
            onBarcode = { raw -> runOnUiThread { handleBarcode(raw) } },
        )
    }

    private val cameraPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            askingForCamera = false
            if (granted) {
                val action = pendingCameraUse
                pendingCameraUse = null
                action?.invoke()
            } else {
                pendingCameraUse = null
                if (screen == Screen.SCAN) {
                    setScanStatus(getString(R.string.camera_needed), true)
                } else {
                    showPairPanel()
                    setPairError(getString(R.string.camera_needed))
                }
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        setContentView(R.layout.activity_main)
        bind()
        scanner = BarcodeScanning.getClient(
            BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_ALL_FORMATS)
                .build(),
        )

        val saved = Pairing.load(prefs)
        if (saved == null) showPairPanel() else showScan(saved)
    }

    override fun onStart() {
        super.onStart()
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        when {
            screen == Screen.SCAN -> requireCamera {
                if (screen == Screen.SCAN) startCamera()
            }
            pairingHint.visibility == View.VISIBLE -> requireCamera {
                if (pairingHint.visibility == View.VISIBLE) startCamera()
            }
        }
    }

    override fun onStop() {
        stopCamera()
        super.onStop()
    }

    override fun onDestroy() {
        stopCamera()
        cameraExecutor.shutdown()
        scanner.close()
        client?.close()
        super.onDestroy()
    }

    private fun bind() {
        previewView = findViewById(R.id.preview)
        topBar = findViewById(R.id.topBar)
        endpointText = findViewById(R.id.endpointText)
        statusText = findViewById(R.id.statusText)
        scanControls = findViewById(R.id.scanControls)
        modeGroup = findViewById(R.id.modeGroup)
        modeSell = findViewById(R.id.modeSell)
        modeAdd = findViewById(R.id.modeAdd)

        setupPanel = findViewById(R.id.setupPanel)
        hostInput = findViewById(R.id.hostInput)
        portInput = findViewById(R.id.portInput)
        tokenInput = findViewById(R.id.tokenInput)
        pairStatus = findViewById(R.id.pairStatus)
        scanQrButton = findViewById(R.id.scanQrButton)
        connectButton = findViewById(R.id.connectButton)
        cancelPairButton = findViewById(R.id.cancelPairButton)
        pairingHint = findViewById(R.id.pairingHint)
        pairingHintText = findViewById(R.id.pairingHintText)

        findViewById<Button>(R.id.repairButton).setOnClickListener { showPairPanel() }
        scanQrButton.setOnClickListener { beginPairingScan() }
        connectButton.setOnClickListener { connectManually() }
        cancelPairButton.setOnClickListener { showPairPanel() }

        modeGroup.setOnCheckedChangeListener { _, checked ->
            prefs.edit()
                .putString(KEY_MODE, if (checked == R.id.modeAdd) "add" else "sell")
                .apply()
        }
        if (prefs.getString(KEY_MODE, "sell") == "add") {
            modeAdd.isChecked = true
        } else {
            modeSell.isChecked = true
        }
    }

    private fun showPairPanel() {
        connectAttempt++
        screen = Screen.PAIR
        cameraMode = Mode.NONE
        stopCamera()

        client?.close()
        client = null

        setupPanel.visibility = View.VISIBLE
        pairingHint.visibility = View.GONE
        topBar.visibility = View.GONE
        scanControls.visibility = View.GONE
        pairStatus.visibility = View.GONE
        setPairBusy(false)

        if (hostInput.text.isNullOrBlank()) {
            Pairing.load(prefs)?.let(::fillPairing)
        }
    }

    private fun showScan(pairing: Pairing) {
        connectAttempt++
        screen = Screen.SCAN
        cameraMode = Mode.SCANNING

        fillPairing(pairing)
        setupPanel.visibility = View.GONE
        pairingHint.visibility = View.GONE
        topBar.visibility = View.VISIBLE
        scanControls.visibility = View.VISIBLE
        endpointText.text = pairing.endpoint
        lastBarcode = ""
        lastScanAt = 0

        client?.close()
        client = TillClient(pairing)
        setScanStatus(getString(R.string.ready), false)

        requireCamera {
            if (screen == Screen.SCAN) startCamera()
        }
    }

    private fun beginPairingScan() {
        if (screen != Screen.PAIR) return
        pairStatus.visibility = View.GONE
        setupPanel.visibility = View.GONE
        pairingHint.visibility = View.VISIBLE
        pairingHintText.text = getString(R.string.pair_scan_hint)
        cameraMode = Mode.PAIRING

        requireCamera {
            if (screen == Screen.PAIR && pairingHint.visibility == View.VISIBLE) startCamera()
        }
    }

    private fun handlePairingCode(raw: String) {
        if (screen != Screen.PAIR || pairingHint.visibility != View.VISIBLE) return

        val pairing = PairingCode.parse(raw)
        if (pairing == null) {
            pairingHintText.text = getString(R.string.pair_bad_qr)
            return
        }

        fillPairing(pairing)
        testAndSave(pairing)
    }

    private fun connectManually() {
        val host = hostInput.text.toString().trim()
            .removePrefix("http://")
            .removePrefix("https://")
            .trimEnd('/')
        val port = portInput.text.toString().trim().toIntOrNull()
        val token = tokenInput.text.toString().trim().lowercase()

        when {
            host.isEmpty() || host.any { it.isWhitespace() } -> {
                setPairError(getString(R.string.error_host))
                return
            }
            port == null || port !in 1..65535 -> {
                setPairError(getString(R.string.error_port))
                return
            }
            !Pairing.validToken(token) -> {
                setPairError(getString(R.string.error_token))
                return
            }
        }

        hostInput.setText(host)
        tokenInput.setText(token)
        testAndSave(Pairing(host, port, token))
    }

    private fun testAndSave(pairing: Pairing) {
        val attempt = ++connectAttempt
        val setupVisible = setupPanel.visibility == View.VISIBLE
        setPairBusy(true)
        if (setupVisible) {
            setPairMessage(getString(R.string.connecting), false)
        } else {
            pairingHintText.text = getString(R.string.connecting)
        }

        // A probe is thrown away on failure; the client that stays is the one
        // behind a connection that actually answered. Nothing is saved until
        // the till proves it is there.
        val probe = TillClient(pairing)
        probe.ping { result ->
            runOnUiThread {
                if (attempt != connectAttempt) {
                    probe.close()
                    return@runOnUiThread
                }
                setPairBusy(false)
                result.onSuccess {
                    probe.close()
                    pairing.save(prefs)
                    showScan(pairing)
                }.onFailure { error ->
                    probe.close()
                    showPairPanel()
                    setPairError(error.message ?: getString(R.string.error_connect))
                }
            }
        }
    }

    private fun handleBarcode(raw: String) {
        if (screen != Screen.SCAN) return

        val now = SystemClock.elapsedRealtime()
        if (raw == lastBarcode && now - lastScanAt < DUPLICATE_WINDOW_MS) return
        lastBarcode = raw
        lastScanAt = now

        val add = modeAdd.isChecked
        val action = if (add) "add" else "sell"
        setScanStatus(getString(R.string.sending, raw), false)
        thump()

        val till = client
        if (till == null) {
            setScanStatus(getString(R.string.not_paired), true)
            return
        }

        till.send(raw, action) { result ->
            runOnUiThread {
                result.onSuccess {
                    val message = if (add) R.string.sent_add else R.string.sent_sell
                    setScanStatus(getString(message, raw), false)
                }.onFailure { error ->
                    setScanStatus(error.message ?: getString(R.string.error_send), true)
                }
            }
        }
    }

    private fun requireCamera(action: () -> Unit) {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            action()
            return
        }
        pendingCameraUse = action
        if (!askingForCamera) {
            askingForCamera = true
            cameraPermission.launch(Manifest.permission.CAMERA)
        }
    }

    private fun startCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            val cameraProvider = try {
                future.get()
            } catch (error: Exception) {
                cameraError(error.message ?: getString(R.string.error_camera))
                return@addListener
            }
            provider = cameraProvider

            val preview = Preview.Builder().build().also {
                it.setSurfaceProvider(previewView.surfaceProvider)
            }
            val analysis = ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .also { it.setAnalyzer(cameraExecutor, analyzer) }

            // A permission dialog and onStart can both ask for the camera. The
            // second bind replaces the first; one unbind keeps that clean.
            if (!lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) return@addListener
            try {
                cameraProvider.unbindAll()
                cameraProvider.bindToLifecycle(
                    this,
                    CameraSelector.DEFAULT_BACK_CAMERA,
                    preview,
                    analysis,
                )
            } catch (error: Exception) {
                cameraError(error.message ?: getString(R.string.error_camera))
            }
        }, ContextCompat.getMainExecutor(this))
    }

    private fun stopCamera() {
        provider?.unbindAll()
    }

    private fun cameraError(message: String) {
        if (screen == Screen.SCAN) {
            setScanStatus(message, true)
        } else {
            showPairPanel()
            setPairError(message)
        }
    }

    private fun fillPairing(pairing: Pairing) {
        hostInput.setText(pairing.host)
        portInput.setText(pairing.port.toString())
        tokenInput.setText(pairing.token)
    }

    private fun setPairMessage(message: String, error: Boolean) {
        pairStatus.text = message
        pairStatus.setTextColor(
            ContextCompat.getColor(this, if (error) R.color.stride_bad else R.color.stride_ink),
        )
        pairStatus.visibility = View.VISIBLE
    }

    private fun setPairError(message: String) {
        setPairMessage(message, true)
    }

    private fun setPairBusy(busy: Boolean) {
        scanQrButton.isEnabled = !busy
        connectButton.isEnabled = !busy
        cancelPairButton.isEnabled = !busy
    }

    private fun setScanStatus(message: String, error: Boolean) {
        statusText.text = message
        statusText.setTextColor(
            ContextCompat.getColor(
                this,
                if (error) R.color.stride_bad_light else R.color.white,
            ),
        )
    }

    @Suppress("DEPRECATION")
    private fun thump() {
        val vibrator = getSystemService(VIBRATOR_SERVICE) as? Vibrator ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            vibrator.vibrate(
                VibrationEffect.createOneShot(35, VibrationEffect.DEFAULT_AMPLITUDE),
            )
        } else {
            vibrator.vibrate(35)
        }
    }

    private enum class Screen { PAIR, SCAN }

    private companion object {
        const val PREFS = "stride-scan"
        const val KEY_MODE = "scan.mode"
        const val DUPLICATE_WINDOW_MS = 1_500L
    }
}
