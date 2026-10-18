package org.omnilink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.omnilink.protocol.ControlPacket

class ControlPacketTest {

    @Test
    fun testPingSerializationMatchesJsonSchema() {
        val ping = ControlPacket.Ping(timestamp = 1729250000000L)
        val jsonStr = ControlPacket.toJson(ping)

        assertTrue(jsonStr.contains("\"Ping\""))
        assertTrue(jsonStr.contains("\"timestamp\":1729250000000"))

        val deserialized = ControlPacket.fromJson(jsonStr)
        assertTrue(deserialized is ControlPacket.Ping)
        assertEquals(1729250000000L, (deserialized as ControlPacket.Ping).timestamp)
    }

    @Test
    fun testPongSerializationMatchesJsonSchema() {
        val pong = ControlPacket.Pong(timestamp = 1729250000100L)
        val jsonStr = ControlPacket.toJson(pong)

        assertTrue(jsonStr.contains("\"Pong\""))
        assertTrue(jsonStr.contains("\"timestamp\":1729250000100"))

        val deserialized = ControlPacket.fromJson(jsonStr)
        assertTrue(deserialized is ControlPacket.Pong)
        assertEquals(1729250000100L, (deserialized as ControlPacket.Pong).timestamp)
    }

    @Test
    fun testDeviceStatusWithWifiSerializationMatchesJsonSchema() {
        val status = ControlPacket.DeviceStatus(
            batteryPct = 85,
            isCharging = true,
            wifiSsid = "Home-5GHz",
            cellularSignal = 4,
            hotspotActive = false
        )
        val jsonStr = ControlPacket.toJson(status)

        assertTrue(jsonStr.contains("\"DeviceStatus\""))
        assertTrue(jsonStr.contains("\"battery_pct\":85"))
        assertTrue(jsonStr.contains("\"is_charging\":true"))
        assertTrue(jsonStr.contains("\"wifi_ssid\":\"Home-5GHz\""))
        assertTrue(jsonStr.contains("\"cellular_signal\":4"))
        assertTrue(jsonStr.contains("\"hotspot_active\":false"))

        val deserialized = ControlPacket.fromJson(jsonStr)
        assertTrue(deserialized is ControlPacket.DeviceStatus)
        val casted = deserialized as ControlPacket.DeviceStatus
        assertEquals(85, casted.batteryPct)
        assertTrue(casted.isCharging)
        assertEquals("Home-5GHz", casted.wifiSsid)
        assertEquals(4, casted.cellularSignal)
        assertEquals(false, casted.hotspotActive)
    }

    @Test
    fun testDeviceStatusWithNullWifiSerializationMatchesJsonSchema() {
        val status = ControlPacket.DeviceStatus(
            batteryPct = 15,
            isCharging = false,
            wifiSsid = null,
            cellularSignal = 1,
            hotspotActive = true
        )
        val jsonStr = ControlPacket.toJson(status)

        assertTrue(jsonStr.contains("\"DeviceStatus\""))
        assertTrue(jsonStr.contains("\"battery_pct\":15"))
        assertTrue(jsonStr.contains("\"is_charging\":false"))
        assertTrue(jsonStr.contains("\"wifi_ssid\":null"))
        assertTrue(jsonStr.contains("\"cellular_signal\":1"))
        assertTrue(jsonStr.contains("\"hotspot_active\":true"))

        val deserialized = ControlPacket.fromJson(jsonStr)
        assertTrue(deserialized is ControlPacket.DeviceStatus)
        val casted = deserialized as ControlPacket.DeviceStatus
        assertEquals(15, casted.batteryPct)
        assertEquals(false, casted.isCharging)
        assertNull(casted.wifiSsid)
        assertEquals(1, casted.cellularSignal)
        assertEquals(true, casted.hotspotActive)
    }

    @Test
    fun testDeserializeExternallyTaggedFromDaemon() {
        val daemonPingJson = """{"Ping":{"timestamp":1729255555000}}"""
        val packet = ControlPacket.fromJson(daemonPingJson)
        assertTrue(packet is ControlPacket.Ping)
        assertEquals(1729255555000L, (packet as ControlPacket.Ping).timestamp)

        val daemonStatusJson = """{"DeviceStatus":{"battery_pct":77,"is_charging":true,"wifi_ssid":"Linux-Desktop-AP","cellular_signal":3,"hotspot_active":false}}"""
        val statusPacket = ControlPacket.fromJson(daemonStatusJson)
        assertTrue(statusPacket is ControlPacket.DeviceStatus)
        val status = statusPacket as ControlPacket.DeviceStatus
        assertEquals(77, status.batteryPct)
        assertTrue(status.isCharging)
        assertEquals("Linux-Desktop-AP", status.wifiSsid)
    }

    @Test
    fun testDeserializeCommandPacket() {
        val commandJson = """{"Command":{"action":"ToggleHotspot"}}"""
        val packet = ControlPacket.fromJson(commandJson)
        assertTrue(packet is ControlPacket.Command)
        assertEquals("ToggleHotspot", (packet as ControlPacket.Command).action)
    }

    @Test
    fun testDeserializeFlatFallback() {
        val flatJson = """{"type":"Ping","timestamp":999999}"""
        val packet = ControlPacket.fromJson(flatJson)
        assertTrue(packet is ControlPacket.Ping)
        assertEquals(999999L, (packet as ControlPacket.Ping).timestamp)
    }
}
