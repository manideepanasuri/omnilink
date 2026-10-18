package org.omnilink.ble

import android.Manifest
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import java.util.UUID

/**
 * Event triggers that prompt proximity BLE advertisement.
 */
enum class BleTriggerEvent {
    SCREEN_ON,
    WIFI_CHANGED
}

/**
 * BroadcastReceiver listening for Screen On and Wi-Fi connectivity transitions
 * to trigger BLE proximity advertisement beacons.
 */
class BleProximityTriggerReceiver(
    private val onTrigger: (BleTriggerEvent) -> Unit
) : BroadcastReceiver() {

    override fun onReceive(context: Context?, intent: Intent?) {
        val action = intent?.action ?: return
        handleAction(action)?.let { onTrigger(it) }
    }

    companion object {
        fun handleAction(action: String?): BleTriggerEvent? {
            return when (action) {
                Intent.ACTION_SCREEN_ON -> BleTriggerEvent.SCREEN_ON
                WifiManager.NETWORK_STATE_CHANGED_ACTION,
                @Suppress("DEPRECATION")
                ConnectivityManager.CONNECTIVITY_ACTION -> BleTriggerEvent.WIFI_CHANGED
                else -> null
            }
        }

        fun createIntentFilter(): IntentFilter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(WifiManager.NETWORK_STATE_CHANGED_ACTION)
            @Suppress("DEPRECATION")
            addAction(ConnectivityManager.CONNECTIVITY_ACTION)
        }
    }
}

/**
 * Ephemeral payload transmitted inside BLE Service Data.
 */
data class BleAdvertisePayload(
    val serviceUuid: UUID,
    val token: String,
    val tokenBytes: ByteArray
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is BleAdvertisePayload) return false
        return serviceUuid == other.serviceUuid &&
                token == other.token &&
                tokenBytes.contentEquals(other.tokenBytes)
    }

    override fun hashCode(): Int {
        var result = serviceUuid.hashCode()
        result = 31 * result + token.hashCode()
        result = 31 * result + tokenBytes.contentHashCode()
        return result
    }
}

/**
 * Advertising state model.
 */
sealed class BleAdvertisingState {
    object Idle : BleAdvertisingState()
    data class Advertising(
        val token: String,
        val advertiseMode: Int,
        val isBurst: Boolean
    ) : BleAdvertisingState()
    data class Error(val message: String, val errorCode: Int? = null) : BleAdvertisingState()
}

/**
 * Callback interface for BLE advertising lifecycle events.
 */
interface BleAdvertiseCallback {
    fun onSuccess()
    fun onFailure(errorCode: Int, message: String)
    fun onStopped()
}

/**
 * Driver abstraction over Android BluetoothLeAdvertiser for testability.
 */
interface BleAdvertiserDriver {
    fun startAdvertising(
        payload: BleAdvertisePayload,
        advertiseMode: Int,
        callback: BleAdvertiseCallback
    )
    fun stopAdvertising(callback: BleAdvertiseCallback)
}

/**
 * Standard implementation using Android Framework BluetoothLeAdvertiser.
 */
class AndroidBleAdvertiserDriver(private val context: Context) : BleAdvertiserDriver {
    private var activeCallback: AdvertiseCallback? = null

    override fun startAdvertising(
        payload: BleAdvertisePayload,
        advertiseMode: Int,
        callback: BleAdvertiseCallback
    ) {
        val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        val advertiser = manager?.adapter?.bluetoothLeAdvertiser
        if (advertiser == null) {
            callback.onFailure(
                AdvertiseCallback.ADVERTISE_FAILED_FEATURE_UNSUPPORTED,
                "BluetoothLeAdvertiser not available or Bluetooth is disabled"
            )
            return
        }

        val settings = BleProximityAdvertiser.buildAdvertiseSettings(advertiseMode)
        val data = BleProximityAdvertiser.buildAdvertiseData(payload)

        val cb = object : AdvertiseCallback() {
            override fun onStartSuccess(settingsInEffect: AdvertiseSettings?) {
                Log.i(TAG, "BLE proximity advertising started successfully (mode=$advertiseMode)")
                callback.onSuccess()
            }

            override fun onStartFailure(errorCode: Int) {
                val msg = BleProximityAdvertiser.errorMessageForCode(errorCode)
                Log.w(TAG, "BLE proximity advertising failed: $msg ($errorCode)")
                callback.onFailure(errorCode, msg)
            }
        }
        activeCallback = cb

        try {
            advertiser.startAdvertising(settings, data, cb)
        } catch (e: SecurityException) {
            Log.w(TAG, "SecurityException starting BLE advertising", e)
            callback.onFailure(
                AdvertiseCallback.ADVERTISE_FAILED_INTERNAL_ERROR,
                "SecurityException: BLUETOOTH_ADVERTISE permission required"
            )
        } catch (e: Exception) {
            Log.w(TAG, "Exception starting BLE advertising", e)
            callback.onFailure(
                AdvertiseCallback.ADVERTISE_FAILED_INTERNAL_ERROR,
                e.message ?: "Unknown error"
            )
        }
    }

    override fun stopAdvertising(callback: BleAdvertiseCallback) {
        activeCallback?.let { cb ->
            try {
                val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
                manager?.adapter?.bluetoothLeAdvertiser?.stopAdvertising(cb)
            } catch (e: Exception) {
                Log.w(TAG, "Error stopping BLE advertising", e)
            }
            activeCallback = null
        }
        callback.onStopped()
    }

    companion object {
        private const val TAG = "AndroidBleDriver"
    }
}

/**
 * Android BLE Proximity Advertiser for Omnilink discovery per docs/PROTOCOL.md Section 1.
 *
 * Advertises Omnilink Service UUID 6f6d6e69-6c69-6e6b-0001-000000000000 with ephemeral
 * paired token service data at ADVERTISE_MODE_LOW_POWER and ADVERTISE_TX_POWER_LOW.
 */
class BleProximityAdvertiser(
    private val context: Context,
    private val driver: BleAdvertiserDriver = AndroidBleAdvertiserDriver(context),
    private val permissionChecker: (Context) -> Boolean = { hasAdvertisePermission(it) },
    private val bluetoothChecker: (Context) -> Boolean = { isBluetoothEnabled(it) },
    private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
) {

    private val _isAdvertising = MutableStateFlow(false)
    val isAdvertisingFlow: StateFlow<Boolean> = _isAdvertising.asStateFlow()

    private val _state = MutableStateFlow<BleAdvertisingState>(BleAdvertisingState.Idle)
    val state: StateFlow<BleAdvertisingState> = _state.asStateFlow()

    private var activeToken: String? = null
    private var activeMode: Int = AdvertiseSettings.ADVERTISE_MODE_LOW_POWER
    private var burstJob: Job? = null
    private var triggerReceiver: BroadcastReceiver? = null

    var onStateChangedListener: ((isAdvertising: Boolean, error: String?) -> Unit)? = null

    private val internalCallback = object : BleAdvertiseCallback {
        override fun onSuccess() {
            _isAdvertising.value = true
            val token = activeToken ?: ""
            val isBurst = activeMode == AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY
            _state.value = BleAdvertisingState.Advertising(token, activeMode, isBurst)
            onStateChangedListener?.invoke(true, null)
        }

        override fun onFailure(errorCode: Int, message: String) {
            _isAdvertising.value = false
            activeToken = null
            _state.value = BleAdvertisingState.Error(message, errorCode)
            onStateChangedListener?.invoke(false, message)
        }

        override fun onStopped() {
            _isAdvertising.value = false
            activeToken = null
            _state.value = BleAdvertisingState.Idle
            onStateChangedListener?.invoke(false, null)
        }
    }

    /**
     * Checks if BLE advertising is currently active.
     */
    fun isAdvertising(): Boolean = _isAdvertising.value

    /**
     * Starts BLE proximity advertisement with the paired device's ephemeral token.
     * Uses ADVERTISE_MODE_LOW_POWER by default for <1% battery impact per day.
     */
    @Synchronized
    fun startAdvertising(
        token: String,
        mode: Int = AdvertiseSettings.ADVERTISE_MODE_LOW_POWER
    ): Boolean {
        if (token.isBlank()) {
            val msg = "Ephemeral token must not be blank"
            Log.w(TAG, msg)
            _state.value = BleAdvertisingState.Error(msg)
            onStateChangedListener?.invoke(false, msg)
            return false
        }

        if (!permissionChecker(context)) {
            val msg = "BLUETOOTH_ADVERTISE permission not granted"
            Log.w(TAG, msg)
            _state.value = BleAdvertisingState.Error(msg)
            onStateChangedListener?.invoke(false, msg)
            return false
        }

        if (!bluetoothChecker(context)) {
            val msg = "Bluetooth is disabled or unsupported"
            Log.w(TAG, msg)
            _state.value = BleAdvertisingState.Error(msg)
            onStateChangedListener?.invoke(false, msg)
            return false
        }

        // Check if already advertising with identical parameters
        if (_isAdvertising.value && activeToken == token && activeMode == mode) {
            return true
        }

        burstJob?.cancel()
        burstJob = null

        val payload = createPayload(token)
        activeToken = token
        activeMode = mode

        driver.startAdvertising(payload, mode, internalCallback)
        return true
    }

    /**
     * Starts a high-frequency burst (ADVERTISE_MODE_LOW_LATENCY) for quick desktop discovery
     * upon screen wake, then automatically throttles down to ADVERTISE_MODE_LOW_POWER.
     */
    @Synchronized
    fun startBurstAdvertising(
        token: String,
        burstDurationMs: Long = DEFAULT_BURST_DURATION_MS
    ): Boolean {
        val started = startAdvertising(token, AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
        if (!started) return false

        burstJob?.cancel()
        burstJob = scope.launch {
            delay(burstDurationMs)
            synchronized(this@BleProximityAdvertiser) {
                if (_isAdvertising.value && activeToken == token) {
                    Log.d(TAG, "Screen-on burst expired; transitioning to LOW_POWER advertising")
                    activeMode = AdvertiseSettings.ADVERTISE_MODE_LOW_POWER
                    val payload = createPayload(token)
                    driver.stopAdvertising(object : BleAdvertiseCallback {
                        override fun onSuccess() {}
                        override fun onFailure(errorCode: Int, message: String) {}
                        override fun onStopped() {
                            driver.startAdvertising(
                                payload,
                                AdvertiseSettings.ADVERTISE_MODE_LOW_POWER,
                                internalCallback
                            )
                        }
                    })
                }
            }
        }
        return true
    }

    /**
     * Safely stops BLE proximity advertising.
     */
    @Synchronized
    fun stopAdvertising() {
        burstJob?.cancel()
        burstJob = null

        if (!_isAdvertising.value && _state.value is BleAdvertisingState.Idle) {
            return
        }

        try {
            driver.stopAdvertising(internalCallback)
        } catch (e: Exception) {
            Log.w(TAG, "Error stopping BLE advertising", e)
        } finally {
            _isAdvertising.value = false
            activeToken = null
            _state.value = BleAdvertisingState.Idle
            onStateChangedListener?.invoke(false, null)
            Log.i(TAG, "BLE proximity advertising stopped")
        }
    }

    /**
     * Trigger helper for screen-on event.
     */
    fun onScreenOn(token: String) {
        startBurstAdvertising(token)
    }

    /**
     * Trigger helper for Wi-Fi change event.
     */
    fun onWifiChanged(token: String) {
        startAdvertising(token, AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
    }

    /**
     * Registers a BroadcastReceiver to automatically trigger BLE advertising on screen-on or Wi-Fi change.
     */
    @Synchronized
    fun registerTriggers(context: Context, tokenProvider: () -> String?) {
        if (triggerReceiver != null) return

        val receiver = BleProximityTriggerReceiver { event ->
            val token = tokenProvider()
            if (!token.isNullOrBlank()) {
                when (event) {
                    BleTriggerEvent.SCREEN_ON -> {
                        Log.d(TAG, "Screen ON trigger detected -> starting burst BLE advertisement")
                        onScreenOn(token)
                    }
                    BleTriggerEvent.WIFI_CHANGED -> {
                        Log.d(TAG, "Wi-Fi change trigger detected -> refreshing BLE advertisement")
                        onWifiChanged(token)
                    }
                }
            }
        }
        triggerReceiver = receiver
        androidx.core.content.ContextCompat.registerReceiver(
            context,
            receiver,
            BleProximityTriggerReceiver.createIntentFilter(),
            androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED
        )
        Log.i(TAG, "Registered BLE proximity trigger receiver")
    }

    /**
     * Unregisters the trigger BroadcastReceiver.
     */
    @Synchronized
    fun unregisterTriggers(context: Context) {
        triggerReceiver?.let { receiver ->
            try {
                context.unregisterReceiver(receiver)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to unregister BLE trigger receiver", e)
            }
            triggerReceiver = null
            Log.i(TAG, "Unregistered BLE proximity trigger receiver")
        }
    }

    companion object {
        private const val TAG = "BleProximityAdvertiser"

        /**
         * Omnilink Service UUID per docs/PROTOCOL.md Section 1.
         */
        const val OMNILINK_SERVICE_UUID_STRING = "6f6d6e69-6c69-6e6b-0001-000000000000"
        val SERVICE_UUID: UUID = UUID.fromString(OMNILINK_SERVICE_UUID_STRING)
        val SERVICE_PARCEL_UUID: ParcelUuid get() = ParcelUuid(SERVICE_UUID)

        const val DEFAULT_BURST_DURATION_MS = 10_000L

        /**
         * Creates an advertising payload containing the Omnilink Service UUID
         * and the paired ephemeral token bytes truncated to 8 bytes for 31-byte MTU compliance.
         */
        fun createPayload(token: String): BleAdvertisePayload {
            require(token.isNotBlank()) { "Ephemeral token must not be blank" }
            val cleanToken = token.trim()
            val rawBytes = if (cleanToken.length >= 16 && cleanToken.length % 2 == 0 && cleanToken.all { it.isDigit() || it in 'a'..'f' || it in 'A'..'F' }) {
                cleanToken.chunked(2).take(8).map { it.toInt(16).toByte() }.toByteArray()
            } else {
                cleanToken.toByteArray(Charsets.UTF_8).take(8).toByteArray()
            }
            return BleAdvertisePayload(
                serviceUuid = SERVICE_UUID,
                token = token,
                tokenBytes = rawBytes
            )
        }

        /**
         * Builds AdvertiseSettings configured for low power consumption (<1% per day).
         */
        fun buildAdvertiseSettings(
            mode: Int = AdvertiseSettings.ADVERTISE_MODE_LOW_POWER
        ): AdvertiseSettings {
            return AdvertiseSettings.Builder()
                .setAdvertiseMode(mode)
                .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_LOW)
                .setConnectable(true)
                .setTimeout(0)
                .build()
        }

        /**
         * Builds AdvertiseData containing the Omnilink Service Data.
         * Omits redundant addServiceUuid to guarantee staying within 31-byte legacy MTU limit.
         */
        fun buildAdvertiseData(payload: BleAdvertisePayload): AdvertiseData {
            val parcelUuid = ParcelUuid(payload.serviceUuid)
            return AdvertiseData.Builder()
                .addServiceData(parcelUuid, payload.tokenBytes)
                .setIncludeDeviceName(false)
                .setIncludeTxPowerLevel(false)
                .build()
        }

        /**
         * Checks whether runtime/install BLE advertise permissions are granted.
         */
        fun hasAdvertisePermission(context: Context): Boolean {
            return try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    ContextCompat.checkSelfPermission(
                        context,
                        Manifest.permission.BLUETOOTH_ADVERTISE
                    ) == PackageManager.PERMISSION_GRANTED
                } else {
                    ContextCompat.checkSelfPermission(
                        context,
                        Manifest.permission.BLUETOOTH_ADMIN
                    ) == PackageManager.PERMISSION_GRANTED &&
                    ContextCompat.checkSelfPermission(
                        context,
                        Manifest.permission.BLUETOOTH
                    ) == PackageManager.PERMISSION_GRANTED
                }
            } catch (_: Exception) {
                false
            }
        }

        /**
         * Checks if the device Bluetooth adapter is present and enabled.
         */
        fun isBluetoothEnabled(context: Context): Boolean {
            return try {
                val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
                manager?.adapter?.isEnabled == true
            } catch (_: Exception) {
                false
            }
        }

        /**
         * Maps AdvertiseCallback failure codes to descriptive error messages.
         */
        fun errorMessageForCode(errorCode: Int): String {
            return when (errorCode) {
                AdvertiseCallback.ADVERTISE_FAILED_ALREADY_STARTED -> "Advertising already started"
                AdvertiseCallback.ADVERTISE_FAILED_DATA_TOO_LARGE -> "Advertise data too large (>31 bytes)"
                AdvertiseCallback.ADVERTISE_FAILED_FEATURE_UNSUPPORTED -> "BLE advertising feature unsupported"
                AdvertiseCallback.ADVERTISE_FAILED_INTERNAL_ERROR -> "Internal BLE advertising error"
                AdvertiseCallback.ADVERTISE_FAILED_TOO_MANY_ADVERTISERS -> "Too many advertisers running"
                else -> "BLE advertise failed with code: $errorCode"
            }
        }
    }
}
