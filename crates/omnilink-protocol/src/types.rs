//! Core protocol types and data structures.

use serde::{Deserialize, Serialize};

/// High-level device telemetry status reported by the Android companion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatus {
    /// Battery charge percentage (0-100).
    pub battery_pct: u8,
    /// Indicates whether the device is currently connected to a charger.
    pub is_charging: bool,
    /// SSID of the connected Wi-Fi network, if available.
    pub wifi_ssid: Option<String>,
    /// Cellular signal strength represented as 0 to 4 bars.
    pub cellular_signal: u8,
    /// Indicates whether the mobile hotspot is currently enabled.
    pub hotspot_active: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_status_serialization() {
        let status = DeviceStatus {
            battery_pct: 85,
            is_charging: true,
            wifi_ssid: Some("Home-5GHz".to_string()),
            cellular_signal: 4,
            hotspot_active: false,
        };

        let json = serde_json::to_string(&status).expect("Failed to serialize DeviceStatus");
        let deserialized: DeviceStatus =
            serde_json::from_str(&json).expect("Failed to deserialize DeviceStatus");

        assert_eq!(status, deserialized);
    }
}
