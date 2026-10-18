package org.omnilink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.omnilink.protocol.DeviceTelemetryInfo

class DeviceTelemetryTest {

    @Test
    fun testBatteryPctClamping() {
        assertEquals(0, DeviceTelemetryInfo.clampBatteryPct(-10))
        assertEquals(0, DeviceTelemetryInfo.clampBatteryPct(0))
        assertEquals(42, DeviceTelemetryInfo.clampBatteryPct(42))
        assertEquals(100, DeviceTelemetryInfo.clampBatteryPct(100))
        assertEquals(100, DeviceTelemetryInfo.clampBatteryPct(120))
    }

    @Test
    fun testCellularSignalClamping() {
        assertEquals(0, DeviceTelemetryInfo.clampCellularSignal(-1))
        assertEquals(0, DeviceTelemetryInfo.clampCellularSignal(0))
        assertEquals(3, DeviceTelemetryInfo.clampCellularSignal(3))
        assertEquals(4, DeviceTelemetryInfo.clampCellularSignal(4))
        assertEquals(4, DeviceTelemetryInfo.clampCellularSignal(10))
    }

    @Test
    fun testWifiSsidSanitization() {
        // Quoted SSID from Android WifiInfo
        assertEquals("HomeNetwork", DeviceTelemetryInfo.sanitizeSsid("\"HomeNetwork\""))
        assertEquals("CoffeeShop", DeviceTelemetryInfo.sanitizeSsid("CoffeeShop"))

        // Unknown / invalid SSIDs
        assertNull(DeviceTelemetryInfo.sanitizeSsid(null))
        assertNull(DeviceTelemetryInfo.sanitizeSsid(""))
        assertNull(DeviceTelemetryInfo.sanitizeSsid("   "))
        assertNull(DeviceTelemetryInfo.sanitizeSsid("<unknown ssid>"))
        assertNull(DeviceTelemetryInfo.sanitizeSsid("\"<unknown ssid>\""))
        assertNull(DeviceTelemetryInfo.sanitizeSsid("0x"))
    }

    @Test
    fun testToDeviceStatusPacket() {
        val info = DeviceTelemetryInfo(
            batteryPct = 105, // should clamp to 100
            isCharging = true,
            wifiSsid = "\"MyOfficeWiFi\"",
            cellularSignal = 5, // should clamp to 4
            hotspotActive = false
        )

        val packet = info.toDeviceStatusPacket()
        assertEquals(100, packet.batteryPct)
        assertTrue(packet.isCharging)
        assertEquals("MyOfficeWiFi", packet.wifiSsid)
        assertEquals(4, packet.cellularSignal)
        assertFalse(packet.hotspotActive)
    }

    @Test
    fun testToDeviceStatusPacketWithNoWifi() {
        val info = DeviceTelemetryInfo(
            batteryPct = 25,
            isCharging = false,
            wifiSsid = "<unknown ssid>",
            cellularSignal = 2,
            hotspotActive = true
        )

        val packet = info.toDeviceStatusPacket()
        assertEquals(25, packet.batteryPct)
        assertFalse(packet.isCharging)
        assertNull(packet.wifiSsid)
        assertEquals(2, packet.cellularSignal)
        assertTrue(packet.hotspotActive)
    }
}
