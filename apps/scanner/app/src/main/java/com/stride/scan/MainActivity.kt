package com.stride.scan

import android.Manifest
import android.app.AlertDialog
import android.content.SharedPreferences
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.core.widget.doAfterTextChanged
import androidx.lifecycle.Lifecycle
import com.google.mlkit.vision.barcode.BarcodeScanner
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlin.math.abs
import org.json.JSONObject

/**
 * One screen, two jobs: pair the phone with a till, then scan for it.
 *
 * There is no account and no cloud session. The only state that outlives the
 * activity is [Pairing] in private preferences, the Sell/Add choice, and the
 * cashier picked for a blackout sale. The blackout till itself is local: a
 * catalogue copy, a cart, and an outbox that retries when the till returns.
 */
class MainActivity : ComponentActivity() {

    private lateinit var prefs: SharedPreferences
    private lateinit var scanner: BarcodeScanner
    private lateinit var db: TillDb

    private lateinit var previewView: PreviewView
    private lateinit var topBar: View
    private lateinit var endpointText: TextView
    private lateinit var statusText: TextView
    private lateinit var syncBadge: TextView
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

    private lateinit var emergencyButton: Button
    private lateinit var emergencyPanel: View
    private lateinit var emergencyBack: Button
    private lateinit var emergencyCashier: Button
    private lateinit var emergencyNotice: TextView
    private lateinit var cartEmpty: TextView
    private lateinit var cartItems: LinearLayout
    private lateinit var emergencyTotal: TextView
    private lateinit var emergencyRounding: TextView
    private lateinit var emergencyTax: TextView
    private lateinit var emergencyDue: TextView
    private lateinit var productSearch: EditText
    private lateinit var searchButton: Button
    private lateinit var searchResults: LinearLayout
    private lateinit var emergencyScanButton: Button
    private lateinit var manualName: EditText
    private lateinit var manualPrice: EditText
    private lateinit var addManualButton: Button
    private lateinit var emergencyStatus: TextView
    private lateinit var rejectedButton: Button
    private lateinit var tenderedInput: EditText
    private lateinit var takeCashButton: Button
    private lateinit var emergencyScanBar: View
    private lateinit var emergencyScanStatus: TextView
    private lateinit var emergencyScanTotal: TextView
    private lateinit var emergencyScanCancel: Button

    private val cameraExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    private var provider: ProcessCameraProvider? = null
    private var client: TillClient? = null
    private var outbox: Outbox? = null
    private var emergencyTill: EmergencyTill? = null

    private val syncHandler = Handler(Looper.getMainLooper())
    private var syncing = false
    private var lastReachable: Boolean? = null
    private var lastSyncError: String? = null

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

    private val syncTick = object : Runnable {
        override fun run() {
            runCatching { runSyncTick() }
            syncHandler.postDelayed(this, SYNC_TICK_MS)
        }
    }

    private val backCallback = object : OnBackPressedCallback(true) {
        override fun handleOnBackPressed() {
            when (screen) {
                Screen.EMERGENCY_SCAN -> resumeEmergencyPanel()
                Screen.EMERGENCY -> showScanner()
                Screen.PAIR ->
                    if (pairingHint.visibility == View.VISIBLE) {
                        showPairPanel()
                    } else {
                        finish()
                    }
                Screen.SCAN -> finish()
            }
        }
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
                } else if (screen == Screen.EMERGENCY_SCAN) {
                    resumeEmergencyPanel(getString(R.string.camera_needed))
                } else {
                    showPairPanel()
                    setPairError(getString(R.string.camera_needed))
                }
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        db = TillDb(applicationContext)
        setContentView(R.layout.activity_main)
        bind()
        scanner = BarcodeScanning.getClient(
            BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_ALL_FORMATS)
                .build(),
        )
        onBackPressedDispatcher.addCallback(this, backCallback)

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
            screen == Screen.EMERGENCY_SCAN -> requireCamera {
                if (screen == Screen.EMERGENCY_SCAN) startCamera()
            }
            pairingHint.visibility == View.VISIBLE -> requireCamera {
                if (pairingHint.visibility == View.VISIBLE) startCamera()
            }
        }
        if (screen != Screen.PAIR) startSyncPolling()
    }

    override fun onStop() {
        stopCamera()
        stopSyncPolling()
        super.onStop()
    }

    override fun onDestroy() {
        stopCamera()
        stopSyncPolling()
        cameraExecutor.shutdown()
        scanner.close()
        client?.close()
        db.close()
        super.onDestroy()
    }

    private fun bind() {
        previewView = findViewById(R.id.preview)
        topBar = findViewById(R.id.topBar)
        endpointText = findViewById(R.id.endpointText)
        statusText = findViewById(R.id.statusText)
        syncBadge = findViewById(R.id.syncBadge)
        scanControls = findViewById(R.id.scanControls)
        modeGroup = findViewById(R.id.modeGroup)
        modeSell = findViewById(R.id.modeSell)
        modeAdd = findViewById(R.id.modeAdd)
        emergencyButton = findViewById(R.id.emergencyButton)

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

        emergencyPanel = findViewById(R.id.emergencyPanel)
        emergencyBack = findViewById(R.id.emergencyBack)
        emergencyCashier = findViewById(R.id.emergencyCashier)
        emergencyNotice = findViewById(R.id.emergencyNotice)
        cartEmpty = findViewById(R.id.cartEmpty)
        cartItems = findViewById(R.id.cartItems)
        emergencyTotal = findViewById(R.id.emergencyTotal)
        emergencyRounding = findViewById(R.id.emergencyRounding)
        emergencyTax = findViewById(R.id.emergencyTax)
        emergencyDue = findViewById(R.id.emergencyDue)
        productSearch = findViewById(R.id.productSearch)
        searchButton = findViewById(R.id.searchButton)
        searchResults = findViewById(R.id.searchResults)
        emergencyScanButton = findViewById(R.id.emergencyScanButton)
        manualName = findViewById(R.id.manualName)
        manualPrice = findViewById(R.id.manualPrice)
        addManualButton = findViewById(R.id.addManualButton)
        emergencyStatus = findViewById(R.id.emergencyStatus)
        rejectedButton = findViewById(R.id.rejectedButton)
        tenderedInput = findViewById(R.id.tenderedInput)
        takeCashButton = findViewById(R.id.takeCashButton)
        emergencyScanBar = findViewById(R.id.emergencyScanBar)
        emergencyScanStatus = findViewById(R.id.emergencyScanStatus)
        emergencyScanTotal = findViewById(R.id.emergencyScanTotal)
        emergencyScanCancel = findViewById(R.id.emergencyScanCancel)

        findViewById<Button>(R.id.repairButton).setOnClickListener { showPairPanel() }
        scanQrButton.setOnClickListener { beginPairingScan() }
        connectButton.setOnClickListener { connectManually() }
        cancelPairButton.setOnClickListener { showPairPanel() }

        emergencyButton.setOnClickListener { showEmergency() }
        emergencyBack.setOnClickListener { showScanner() }
        emergencyCashier.setOnClickListener { chooseCashier() }
        searchButton.setOnClickListener { searchProducts() }
        emergencyScanButton.setOnClickListener { beginEmergencyScan() }
        addManualButton.setOnClickListener { addManual() }
        takeCashButton.setOnClickListener { takeCash() }
        rejectedButton.setOnClickListener { showRejectedDialog() }
        emergencyScanCancel.setOnClickListener { resumeEmergencyPanel() }

        productSearch.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_SEARCH) {
                searchProducts()
                true
            } else {
                false
            }
        }
        manualPrice.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_DONE) {
                addManual()
                true
            } else {
                false
            }
        }
        tenderedInput.doAfterTextChanged { updatePayableState() }

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
        updateSyncBadge()
    }

    private fun showPairPanel() {
        connectAttempt++
        screen = Screen.PAIR
        cameraMode = Mode.NONE
        stopCamera()
        stopSyncPolling()

        client?.close()
        client = null
        outbox = null

        setupPanel.visibility = View.VISIBLE
        pairingHint.visibility = View.GONE
        topBar.visibility = View.GONE
        scanControls.visibility = View.GONE
        emergencyPanel.visibility = View.GONE
        emergencyScanBar.visibility = View.GONE
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
        emergencyPanel.visibility = View.GONE
        emergencyScanBar.visibility = View.GONE
        topBar.visibility = View.VISIBLE
        scanControls.visibility = View.VISIBLE
        endpointText.text = pairing.endpoint
        lastBarcode = ""
        lastScanAt = 0

        client?.close()
        client = TillClient(pairing)
        outbox = Outbox(db, client!!)
        setScanStatus(getString(R.string.ready), false)
        updateSyncBadge()
        startSyncPolling()

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

        when {
            host.isEmpty() || host.any { it.isWhitespace() } -> {
                setPairError(getString(R.string.error_host))
                return
            }
            port == null || port !in 1..65535 -> {
                setPairError(getString(R.string.error_port))
                return
            }
        }

        val token = Pairing.normalize(tokenInput.text.toString())
        if (token == null) {
            setPairError(getString(R.string.error_token))
            return
        }

        hostInput.setText(host)
        tokenInput.setText(Pairing.display(token))
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
        val now = SystemClock.elapsedRealtime()
        if (raw == lastBarcode && now - lastScanAt < DUPLICATE_WINDOW_MS) return
        lastBarcode = raw
        lastScanAt = now

        when (screen) {
            Screen.SCAN -> sendScan(raw)
            Screen.EMERGENCY_SCAN -> addScanned(raw)
            else -> Unit
        }
    }

    private fun sendScan(raw: String) {
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

    // ---- blackout till -------------------------------------------------

    private fun showEmergency() {
        if (screen != Screen.SCAN) return

        val till = emergencyTill ?: EmergencyTill(db).also {
            emergencyTill = it
            it.cashier = loadCashier()
        }
        till.refresh()

        screen = Screen.EMERGENCY
        cameraMode = Mode.NONE
        stopCamera()
        topBar.visibility = View.GONE
        scanControls.visibility = View.GONE
        emergencyScanBar.visibility = View.GONE
        emergencyPanel.visibility = View.VISIBLE
        emergencyStatus.visibility = View.GONE

        renderEmergency()
        runSyncTick()
    }

    private fun showScanner() {
        screen = Screen.SCAN
        cameraMode = Mode.SCANNING
        stopCamera()
        emergencyPanel.visibility = View.GONE
        emergencyScanBar.visibility = View.GONE
        topBar.visibility = View.VISIBLE
        scanControls.visibility = View.VISIBLE
        lastBarcode = ""
        lastScanAt = 0
        setScanStatus(getString(R.string.ready), false)
        updateSyncBadge()
        startSyncPolling()

        requireCamera {
            if (screen == Screen.SCAN) startCamera()
        }
    }

    private fun beginEmergencyScan() {
        if (screen != Screen.EMERGENCY) return
        screen = Screen.EMERGENCY_SCAN
        cameraMode = Mode.EMERGENCY
        emergencyPanel.visibility = View.GONE
        emergencyScanBar.visibility = View.VISIBLE
        emergencyScanStatus.text = getString(R.string.emergency_scan_hint)
        renderScanBar()
        lastBarcode = ""
        lastScanAt = 0

        requireCamera {
            if (screen == Screen.EMERGENCY_SCAN) startCamera()
        }
    }

    private fun resumeEmergencyPanel(message: String? = null) {
        if (screen != Screen.EMERGENCY_SCAN && screen != Screen.EMERGENCY) return
        screen = Screen.EMERGENCY
        cameraMode = Mode.NONE
        stopCamera()
        emergencyScanBar.visibility = View.GONE
        emergencyPanel.visibility = View.VISIBLE
        message?.let { setEmergencyStatus(it, true) }
        renderEmergency()
    }

    private fun addScanned(raw: String) {
        val till = emergencyTill ?: return
        thump()

        val product = runCatching { db.productByBarcode(raw) }.getOrNull()
        if (product == null) {
            emergencyScanStatus.text = getString(R.string.emergency_not_in_catalog)
            return
        }

        till.addProduct(product)
        emergencyScanStatus.text = getString(R.string.emergency_added, product.name)
        renderScanBar()
    }

    private fun renderScanBar() {
        emergencyScanTotal.text = getString(
            R.string.emergency_scan_total,
            money(emergencyTill?.totalMinor() ?: 0),
        )
    }

    private fun renderEmergency() {
        renderCart()
        updateCashierButton()
        updateRejectedButton()
        renderEmergencyNotice()
        updateSyncBadge()
    }

    private fun renderCart() {
        val till = emergencyTill ?: return
        cartItems.removeAllViews()

        val items = till.cart.items
        cartEmpty.visibility = if (items.isEmpty()) View.VISIBLE else View.GONE
        for (line in items) cartItems.addView(cartRow(line, till))

        emergencyTotal.text = money(till.totalMinor())
        emergencyRounding.text = signedMoney(till.roundingMinor())
        emergencyTax.text = money(till.taxMinor())
        emergencyDue.text = money(till.dueMinor())
        updatePayableState()
        renderScanBar()
    }

    private fun cartRow(line: CartLine, till: EmergencyTill): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, dp(6), 0, dp(6))
        }

        val name = TextView(this).apply {
            text = line.name
            textSize = 14f
            maxLines = 2
            setTextColor(ContextCompat.getColor(this@MainActivity, R.color.stride_ink))
        }
        row.addView(name, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))

        row.addView(
            smallButton("\u2212") {
                till.cart.change(line.uid, -1)
                renderCart()
            },
        )
        val qty = TextView(this).apply {
            text = line.qty.toString()
            textSize = 14f
            gravity = Gravity.CENTER
            setTextColor(ContextCompat.getColor(this@MainActivity, R.color.stride_ink))
        }
        row.addView(qty, LinearLayout.LayoutParams(dp(28), ViewGroup.LayoutParams.WRAP_CONTENT))
        row.addView(
            smallButton("+") {
                till.cart.change(line.uid, 1)
                renderCart()
            },
        )

        val total = TextView(this).apply {
            text = money(line.totalMinor)
            textSize = 14f
            gravity = Gravity.END
            setPadding(dp(8), 0, 0, 0)
            setTextColor(ContextCompat.getColor(this@MainActivity, R.color.stride_ink))
        }
        row.addView(total, LinearLayout.LayoutParams(dp(88), ViewGroup.LayoutParams.WRAP_CONTENT))
        return row
    }

    private fun smallButton(label: String, action: () -> Unit): Button =
        Button(this, null, android.R.attr.borderlessButtonStyle).apply {
            text = label
            isAllCaps = false
            minWidth = 0
            minimumWidth = 0
            setPadding(dp(10), 0, dp(10), 0)
            layoutParams = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                dp(36),
            )
            setOnClickListener { action() }
        }

    private fun updatePayableState() {
        val till = emergencyTill
        if (till == null) {
            takeCashButton.isEnabled = false
            return
        }
        val raw = tenderedInput.text.toString().trim()
        val due = till.dueMinor()
        val cash = if (raw.isEmpty()) due else MoneyText.parse(raw, till.settings.exponent)
        takeCashButton.isEnabled = !till.cart.isEmpty && cash != null && cash >= due
    }

    private fun searchProducts() {
        val term = productSearch.text.toString().trim()
        searchResults.removeAllViews()
        if (term.isEmpty()) return

        val results = runCatching { db.searchProducts(term) }.getOrDefault(emptyList())
        if (results.isEmpty()) {
            searchResults.addView(
                resultLabel(getString(R.string.emergency_no_match)),
            )
            return
        }

        for (product in results) {
            val button = Button(this, null, android.R.attr.borderlessButtonStyle).apply {
                text = "${product.name}  ${money(product.priceMinor)}"
                isAllCaps = false
                gravity = Gravity.START or Gravity.CENTER_VERTICAL
                setTextColor(ContextCompat.getColor(this@MainActivity, R.color.stride_ink))
                setOnClickListener {
                    emergencyTill?.addProduct(product)
                    productSearch.setText("")
                    searchResults.removeAllViews()
                    setEmergencyStatus(getString(R.string.emergency_added, product.name), false)
                    renderCart()
                }
            }
            searchResults.addView(
                button,
                LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                ),
            )
        }
    }

    private fun resultLabel(text: String): TextView = TextView(this).apply {
        this.text = text
        textSize = 14f
        setPadding(dp(8), dp(8), dp(8), dp(8))
        setTextColor(ContextCompat.getColor(this@MainActivity, R.color.stride_muted))
    }

    private fun addManual() {
        val till = emergencyTill ?: return
        val name = manualName.text.toString().trim()
        if (name.isEmpty()) {
            setEmergencyStatus(getString(R.string.emergency_bad_name), true)
            return
        }

        val price = MoneyText.parse(manualPrice.text.toString(), till.settings.exponent)
        if (price == null || price <= 0) {
            setEmergencyStatus(getString(R.string.emergency_bad_price), true)
            return
        }

        till.addManual(name, price)
        manualName.setText("")
        manualPrice.setText("")
        setEmergencyStatus(getString(R.string.emergency_added, name), false)
        renderCart()
    }

    private fun takeCash() {
        val till = emergencyTill ?: return
        if (till.cart.isEmpty) return

        val raw = tenderedInput.text.toString().trim()
        val tendered = if (raw.isEmpty()) null else MoneyText.parse(raw, till.settings.exponent)
        if (raw.isNotEmpty() && tendered == null) {
            setEmergencyStatus(getString(R.string.emergency_bad_tender), true)
            return
        }

        val due = till.dueMinor()
        if ((tendered ?: due) < due) {
            setEmergencyStatus(getString(R.string.emergency_bad_tender), true)
            return
        }

        val draft = try {
            till.checkout(tendered)
        } catch (error: IllegalArgumentException) {
            setEmergencyStatus(error.message ?: getString(R.string.emergency_bad_tender), true)
            return
        }

        val saved = runCatching {
            db.insertOfflineSale(draft.uid, SaleJson.encode(draft), draft.occurredAt)
        }
        if (saved.isFailure) {
            setEmergencyStatus(getString(R.string.emergency_not_saved), true)
            return
        }

        till.cart.clear()
        tenderedInput.setText("")
        setEmergencyStatus(getString(R.string.emergency_saved), false)
        thump()
        renderCart()
        updateRejectedButton()
        renderEmergencyNotice()

        outbox?.retrySoon()
        flushOutbox()
    }

    private fun chooseCashier() {
        val till = emergencyTill ?: return
        val staff = runCatching { db.staff() }.getOrDefault(emptyList())
        val names = ArrayList<String>(staff.size + 1)
        names.add(getString(R.string.emergency_unassigned))
        for (member in staff) names.add(member.name)

        AlertDialog.Builder(this)
            .setTitle(R.string.emergency_cashier_title)
            .setItems(names.toTypedArray()) { _, index ->
                val picked = if (index == 0) {
                    Cashier.unassigned
                } else {
                    val member = staff[index - 1]
                    Cashier(member.id, member.name)
                }
                till.cashier = picked
                persistCashier(picked)
                updateCashierButton()
            }
            .setNegativeButton(R.string.emergency_close, null)
            .show()
    }

    private fun updateCashierButton() {
        val cashier = emergencyTill?.cashier ?: Cashier.unassigned
        emergencyCashier.text = if (cashier.id == null) {
            getString(R.string.emergency_cashier_unset)
        } else {
            getString(R.string.emergency_cashier, cashier.name)
        }
    }

    private fun persistCashier(cashier: Cashier) {
        val editor = prefs.edit()
        if (cashier.id == null) {
            editor.remove(KEY_CASHIER_ID).remove(KEY_CASHIER_NAME)
        } else {
            editor.putString(KEY_CASHIER_ID, cashier.id).putString(KEY_CASHIER_NAME, cashier.name)
        }
        editor.apply()
    }

    private fun loadCashier(): Cashier {
        val name = prefs.getString(KEY_CASHIER_NAME, null)
            ?.takeIf { it.isNotBlank() }
            ?: return Cashier.unassigned
        val id = prefs.getString(KEY_CASHIER_ID, null)?.takeIf { it.isNotBlank() }
        return Cashier(id, name)
    }

    private fun showRejectedDialog() {
        val sales = runCatching { db.rejectedSales(20) }.getOrDefault(emptyList())
        if (sales.isEmpty()) return

        val body = sales.joinToString("\n\n") { "${it.occurredAt}\n${it.reason}" }
        AlertDialog.Builder(this)
            .setTitle(R.string.emergency_rejected_title)
            .setMessage(body)
            .setPositiveButton(R.string.emergency_retry) { _, _ ->
                runCatching { db.retryRejected() }
                outbox?.retrySoon()
                flushOutbox()
                updateRejectedButton()
                renderEmergencyNotice()
                setEmergencyStatus(
                    getString(R.string.emergency_queued, runCatching { db.pendingCount() }.getOrDefault(0)),
                    false,
                )
            }
            .setNegativeButton(R.string.emergency_close, null)
            .show()
    }

    private fun setEmergencyStatus(message: String, error: Boolean) {
        emergencyStatus.text = message
        emergencyStatus.setTextColor(
            ContextCompat.getColor(this, if (error) R.color.stride_bad else R.color.stride_ink),
        )
        emergencyStatus.visibility = View.VISIBLE
    }

    private fun renderEmergencyNotice() {
        val till = emergencyTill ?: return
        val pending = runCatching { db.pendingCount() }.getOrDefault(0)
        val rejected = runCatching { db.rejectedCount() }.getOrDefault(0)

        val parts = ArrayList<String>()
        if (till.settings.currency.isBlank()) parts.add(getString(R.string.emergency_no_catalog))
        if (pending > 0) parts.add(getString(R.string.emergency_queued, pending))
        if (rejected > 0) parts.add(getString(R.string.sync_rejected, rejected))
        lastSyncError?.let { parts.add(it) }

        emergencyNotice.text = parts.joinToString("\n")
        emergencyNotice.visibility = if (parts.isEmpty()) View.GONE else View.VISIBLE
        emergencyNotice.setTextColor(
            ContextCompat.getColor(
                this,
                if (rejected > 0 || lastSyncError != null) R.color.stride_bad else R.color.stride_muted,
            ),
        )
    }

    private fun updateRejectedButton() {
        val rejected = runCatching { db.rejectedCount() }.getOrDefault(0)
        rejectedButton.visibility = if (rejected > 0) View.VISIBLE else View.GONE
        if (rejected > 0) {
            rejectedButton.text = getString(R.string.emergency_rejected_button, rejected)
        }
    }

    // ---- sync ----------------------------------------------------------

    private fun startSyncPolling() {
        syncHandler.removeCallbacks(syncTick)
        syncHandler.postDelayed(syncTick, FIRST_SYNC_DELAY_MS)
    }

    private fun stopSyncPolling() {
        syncHandler.removeCallbacks(syncTick)
    }

    /**
     * One tick does at most one useful network job.
     *
     * Queued sales come first: they are money sitting on a phone. When the
     * outbox is empty, the catalogue is refreshed only if it is stale. Nothing
     * here runs on the sale path.
     */
    private fun runSyncTick() {
        val till = client ?: return

        val pending = runCatching { db.pendingCount() }.getOrDefault(0)
        if (pending > 0) {
            if (outbox?.due() == true) flushOutbox()
            return
        }

        if (System.currentTimeMillis() - db.lastCatalogAt() < CATALOG_FRESH_MS) return
        if (syncing) return
        syncing = true

        till.info { infoResult ->
            runOnUiThread {
                infoResult.onSuccess { info ->
                    info.serverTime?.let(Clock::parse)?.let { server ->
                        db.putClockOffset(server - System.currentTimeMillis())
                        emergencyTill?.refresh()
                    }
                    lastReachable = true
                    updateSyncBadge()

                    till.fetchCatalog { catalogResult ->
                        runOnUiThread {
                            catalogResult.onSuccess { body ->
                                val applied = runCatching { db.replaceCatalog(JSONObject(body)) }
                                if (applied.isSuccess) {
                                    lastSyncError = null
                                    emergencyTill?.refresh()
                                    if (screen == Screen.EMERGENCY) renderEmergency()
                                } else {
                                    setSyncError(getString(R.string.error_sync))
                                }
                            }.onFailure { error ->
                                setSyncError(error.message ?: getString(R.string.error_sync))
                            }
                            syncing = false
                            updateSyncBadge()
                        }
                    }
                }.onFailure { error ->
                    lastReachable = false
                    syncing = false
                    setSyncError(error.message ?: getString(R.string.sync_offline))
                }
            }
        }
    }

    private fun flushOutbox() {
        val box = outbox ?: return
        box.flush { result ->
            runOnUiThread {
                if (result.busy) return@runOnUiThread

                if (result.error != null) {
                    lastSyncError = result.error
                    lastReachable = false
                } else if (result.accepted > 0 || result.rejected > 0) {
                    lastSyncError = null
                    lastReachable = true
                }
                if (result.accepted > 0) thump()

                updateSyncBadge()
                updateRejectedButton()
                if (screen == Screen.EMERGENCY) {
                    renderEmergencyNotice()
                    if (result.accepted > 0) {
                        setEmergencyStatus(getString(R.string.emergency_saved), false)
                    }
                }
            }
        }
    }

    private fun setSyncError(message: String) {
        lastSyncError = message
        updateSyncBadge()
        if (screen == Screen.EMERGENCY) renderEmergencyNotice()
    }

    private fun updateSyncBadge() {
        val pending = runCatching { db.pendingCount() }.getOrDefault(0)
        val rejected = runCatching { db.rejectedCount() }.getOrDefault(0)

        val text = when {
            rejected > 0 -> getString(R.string.sync_rejected, rejected)
            pending > 0 -> getString(R.string.sync_pending, pending)
            lastReachable == null -> getString(R.string.sync_checking)
            lastReachable == false -> getString(R.string.sync_offline)
            else -> getString(R.string.sync_online)
        }
        val color = when {
            rejected > 0 -> R.color.stride_bad_light
            lastReachable == false -> R.color.stride_bad_light
            else -> R.color.white
        }
        syncBadge.text = text
        syncBadge.setTextColor(ContextCompat.getColor(this, color))
        syncBadge.visibility = View.VISIBLE
    }

    // ---- camera and helpers -------------------------------------------

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
        when (screen) {
            Screen.SCAN -> setScanStatus(message, true)
            Screen.EMERGENCY_SCAN -> resumeEmergencyPanel(message)
            else -> {
                showPairPanel()
                setPairError(message)
            }
        }
    }

    private fun fillPairing(pairing: Pairing) {
        hostInput.setText(pairing.host)
        portInput.setText(pairing.port.toString())
        tokenInput.setText(Pairing.display(pairing.token))
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

    private fun money(minor: Long): String =
        MoneyText.format(minor, emergencyTill?.settings?.exponent ?: 0)

    private fun signedMoney(minor: Long): String = when {
        minor > 0 -> "+" + money(minor)
        minor < 0 -> "-" + money(abs(minor))
        else -> money(0)
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

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

    private enum class Screen { PAIR, SCAN, EMERGENCY, EMERGENCY_SCAN }

    private companion object {
        const val PREFS = "stride-scan"
        const val KEY_MODE = "scan.mode"
        const val KEY_CASHIER_ID = "cashier.id"
        const val KEY_CASHIER_NAME = "cashier.name"
        const val DUPLICATE_WINDOW_MS = 1_500L
        const val FIRST_SYNC_DELAY_MS = 600L
        const val SYNC_TICK_MS = 30_000L
        const val CATALOG_FRESH_MS = 5 * 60_000L
    }
}
