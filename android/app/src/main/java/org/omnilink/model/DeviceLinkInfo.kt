package org.omnilink.model

import kotlinx.serialization.Serializable

enum class ConnectionState {
    DISCONNECTED,
    CONNECTING,
    CONNECTED
}

enum class TransportType {
    QUIC,
    BLE_PROXIMITY,
    USB_FORWARD
}

@Serializable
data class DeviceLinkInfo(
    val deviceName: String = "Linux Desktop",
    val hostAddress: String = "192.168.1.100",
    val port: Int = 42424,
    val connectionState: ConnectionState = ConnectionState.DISCONNECTED,
    val transportType: TransportType = TransportType.QUIC,
    val latencyMs: Long? = null,
    val batteryOptimizationExempt: Boolean = false,
    val shizukuAvailable: Boolean = false,
    val notificationListenerEnabled: Boolean = false
)
