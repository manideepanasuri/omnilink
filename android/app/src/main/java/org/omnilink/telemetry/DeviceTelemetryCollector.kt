package org.omnilink.telemetry

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.telephony.TelephonyManager
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.omnilink.protocol.ControlPacket
import org.omnilink.protocol.DeviceTelemetryInfo

class DeviceTelemetryCollector(private val context: Context) {

    private val _currentStatus = MutableStateFlow(collectCurrentStatus())
    val currentStatus: StateFlow<ControlPacket.DeviceStatus> = _currentStatus.asStateFlow()

    private var batteryReceiver: BroadcastReceiver? = null

    @Synchronized
    fun start(onStatusChanged: (ControlPacket.DeviceStatus) -> Unit) {
        if (batteryReceiver != null) return

        val receiver = object : BroadcastReceiver() {
            override fun onReceive(ctx: Context?, intent: Intent?) {
                if (intent?.action == Intent.ACTION_BATTERY_CHANGED) {
                    val updated = collectCurrentStatus()
                    if (updated != _currentStatus.value) {
                        _currentStatus.value = updated
                        onStatusChanged(updated)
                    }
                }
            }
        }
        batteryReceiver = receiver
        val filter = IntentFilter(Intent.ACTION_BATTERY_CHANGED)
        context.registerReceiver(receiver, filter)

        // Initial snapshot update
        val initial = collectCurrentStatus()
        _currentStatus.value = initial
        onStatusChanged(initial)
    }

    @Synchronized
    fun stop() {
        batteryReceiver?.let {
            try {
                context.unregisterReceiver(it)
            } catch (e: Exception) {
                Log.w(TAG, "Battery receiver unregister failed", e)
            }
            batteryReceiver = null
        }
    }

    fun refreshAndNotify(onStatusChanged: (ControlPacket.DeviceStatus) -> Unit) {
        val updated = collectCurrentStatus()
        if (updated != _currentStatus.value) {
            _currentStatus.value = updated
            onStatusChanged(updated)
        }
    }

    fun collectCurrentStatus(): ControlPacket.DeviceStatus {
        val batteryIntent = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val (batteryPct, isCharging) = parseBatteryIntent(batteryIntent)
        val wifiSsid = queryWifiSsid()
        val cellularSignal = queryCellularSignal()
        val hotspotActive = queryHotspotActive()

        val info = DeviceTelemetryInfo(
            batteryPct = batteryPct,
            isCharging = isCharging,
            wifiSsid = wifiSsid,
            cellularSignal = cellularSignal,
            hotspotActive = hotspotActive
        )
        return info.toDeviceStatusPacket()
    }

    private fun parseBatteryIntent(intent: Intent?): Pair<Int, Boolean> {
        if (intent == null) return Pair(100, false)
        val level = intent.getIntExtra(BatteryManager.EXTRA_LEVEL, -1)
        val scale = intent.getIntExtra(BatteryManager.EXTRA_SCALE, -1)
        val pct = if (level >= 0 && scale > 0) {
            ((level.toFloat() / scale.toFloat()) * 100).toInt()
        } else {
            100
        }
        val status = intent.getIntExtra(BatteryManager.EXTRA_STATUS, -1)
        val isCharging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                status == BatteryManager.BATTERY_STATUS_FULL
        return Pair(pct, isCharging)
    }

    @Suppress("DEPRECATION")
    private fun queryWifiSsid(): String? {
        return try {
            val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            val rawSsid = wifiManager?.connectionInfo?.ssid
            DeviceTelemetryInfo.sanitizeSsid(rawSsid)
        } catch (_: Throwable) {
            null
        }
    }

    private fun queryCellularSignal(): Int {
        return try {
            val tm = context.getSystemService(Context.TELEPHONY_SERVICE) as? TelephonyManager
            tm?.signalStrength?.level ?: 0
        } catch (_: Throwable) {
            0
        }
    }

    private fun queryHotspotActive(): Boolean {
        return try {
            val wifiManager = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
            val method = wifiManager?.javaClass?.getDeclaredMethod("isWifiApEnabled")
            (method?.invoke(wifiManager) as? Boolean) ?: false
        } catch (_: Throwable) {
            false
        }
    }

    companion object {
        private const val TAG = "DeviceTelemetryCollector"
    }
}
