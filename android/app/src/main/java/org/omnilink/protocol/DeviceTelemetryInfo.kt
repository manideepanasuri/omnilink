package org.omnilink.protocol

data class DeviceTelemetryInfo(
    val batteryPct: Int,
    val isCharging: Boolean,
    val wifiSsid: String?,
    val cellularSignal: Int,
    val hotspotActive: Boolean
) {
    fun toDeviceStatusPacket(): ControlPacket.DeviceStatus {
        return ControlPacket.DeviceStatus(
            batteryPct = clampBatteryPct(batteryPct),
            isCharging = isCharging,
            wifiSsid = sanitizeSsid(wifiSsid),
            cellularSignal = clampCellularSignal(cellularSignal),
            hotspotActive = hotspotActive
        )
    }

    companion object {
        fun sanitizeSsid(rawSsid: String?): String? {
            if (rawSsid == null) return null
            val unquoted = rawSsid.removeSurrounding("\"").trim()
            if (unquoted.isEmpty() || unquoted == "<unknown ssid>" || unquoted == "0x") {
                return null
            }
            return unquoted
        }

        fun clampBatteryPct(pct: Int): Int = pct.coerceIn(0, 100)
        fun clampCellularSignal(bars: Int): Int = bars.coerceIn(0, 4)
    }
}
