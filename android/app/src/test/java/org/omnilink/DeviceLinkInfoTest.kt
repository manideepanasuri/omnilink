package org.omnilink

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.omnilink.model.ConnectionState
import org.omnilink.model.DeviceLinkInfo
import org.omnilink.model.TransportType

class DeviceLinkInfoTest {

    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun testDefaultState() {
        val linkInfo = DeviceLinkInfo()
        assertEquals(ConnectionState.DISCONNECTED, linkInfo.connectionState)
        assertEquals(TransportType.QUIC, linkInfo.transportType)
        assertEquals(42424, linkInfo.port)
    }

    @Test
    fun testSerializationDeserialization() {
        val original = DeviceLinkInfo(
            deviceName = "Workstation Hyprland",
            hostAddress = "10.0.0.42",
            port = 42424,
            connectionState = ConnectionState.CONNECTED,
            transportType = TransportType.QUIC,
            latencyMs = 5L,
            batteryOptimizationExempt = true,
            shizukuAvailable = true,
            notificationListenerEnabled = true
        )

        val serialized = json.encodeToString(original)
        assertNotNull(serialized)
        assertTrue(serialized.contains("Workstation Hyprland"))
        assertTrue(serialized.contains("CONNECTED"))

        val deserialized = json.decodeFromString<DeviceLinkInfo>(serialized)
        assertEquals(original, deserialized)
    }

    @Test
    fun testStateTransition() {
        val linkInfo = DeviceLinkInfo()
        val connecting = linkInfo.copy(connectionState = ConnectionState.CONNECTING)
        assertEquals(ConnectionState.CONNECTING, connecting.connectionState)

        val connected = connecting.copy(
            connectionState = ConnectionState.CONNECTED,
            latencyMs = 3L
        )
        assertEquals(ConnectionState.CONNECTED, connected.connectionState)
        assertEquals(3L, connected.latencyMs)
    }
}
