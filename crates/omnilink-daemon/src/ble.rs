//! Linux BLE proximity advertisement scanner for Omnilink.
//!
//! Scans for companion device BLE proximity advertisements using `bluer`
//! and validates ephemeral pairing tokens embedded in advertisement service data.

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc;

/// Omnilink BLE proximity beacon service UUID (128-bit).
///
/// Matches the service UUID advertised by the Omnilink Android companion.
pub const OMNILINK_SERVICE_UUID: uuid::Uuid =
    uuid::uuid!("6f6d6e69-6c69-6e6b-0001-000000000000");

/// Discovered BLE proximity beacon event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BleDiscoveryEvent {
    /// Bluetooth address of the advertising companion device.
    pub device_address: bluer::Address,
    /// Whether the beacon service data matched an expected paired ephemeral token.
    pub token_matched: bool,
}

impl BleDiscoveryEvent {
    /// Creates a new `BleDiscoveryEvent`.
    pub fn new(device_address: bluer::Address, token_matched: bool) -> Self {
        Self {
            device_address,
            token_matched,
        }
    }
}

/// Errors that can occur during BLE scanning operations.
#[derive(Debug, thiserror::Error)]
pub enum BleError {
    /// Low-level Bluetooth / BlueZ error.
    #[error("Bluetooth D-Bus error: {0}")]
    Bluetooth(#[from] bluer::Error),

    /// No Bluetooth adapter was found on this system.
    #[error("No Bluetooth adapter found")]
    NoAdapter,

    /// The specified Bluetooth adapter was not found.
    #[error("Bluetooth adapter '{0}' not found")]
    AdapterNotFound(String),

    /// The Bluetooth adapter could not be powered on.
    #[error("Bluetooth adapter is not powered")]
    AdapterNotPowered,

    /// Scanner event channel was closed.
    #[error("Scanner discovery channel closed")]
    ChannelClosed,
}

/// Compares two byte slices in constant time to mitigate timing side channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Compares two ASCII byte slices in constant time, ignoring case.
fn constant_time_eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x.to_ascii_lowercase() ^ y.to_ascii_lowercase();
    }
    diff == 0
}

/// Decodes an even-length hexadecimal string into raw bytes safely.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.is_ascii() || !hex.len().is_multiple_of(2) {
        return None;
    }
    let (chunks, _) = hex.as_bytes().as_chunks::<2>();
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let s = std::str::from_utf8(chunk).ok()?;
        let byte = u8::from_str_radix(s, 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

/// Verifies whether the BLE advertisement service data matches the expected pairing token.
///
/// Accepts the expected token as direct UTF-8 bytes, raw hex-decoded bytes, or
/// an 8+ byte prefix (to accommodate standard 31-byte legacy BLE advertising MTU limits).
/// Uses constant-time comparisons across all matching paths.
pub fn verify_beacon_payload(service_data: &[u8], expected_token: &str) -> bool {
    let trimmed_token = expected_token.trim();
    if service_data.is_empty() || trimmed_token.is_empty() {
        return false;
    }

    // Strip optional trailing null byte
    let data = match service_data.strip_suffix(b"\0") {
        Some(stripped) => stripped,
        None => service_data,
    };

    if data.is_empty() {
        return false;
    }

    // 1. Direct UTF-8 match or prefix match (minimum 8 bytes)
    let token_bytes = trimmed_token.as_bytes();
    if constant_time_eq(data, token_bytes) {
        return true;
    }
    if data.len() >= 8 && data.len() < token_bytes.len() && constant_time_eq(data, &token_bytes[..data.len()]) {
        return true;
    }

    // 2. Raw bytes match if expected_token is a hex string
    if trimmed_token.len().is_multiple_of(2) {
        if let Some(raw_expected) = decode_hex(trimmed_token) {
            if constant_time_eq(data, &raw_expected) {
                return true;
            }
            if data.len() >= 8 && data.len() < raw_expected.len() && constant_time_eq(data, &raw_expected[..data.len()]) {
                return true;
            }
        }
    }

    // 3. Constant-time ASCII case-insensitive match (full or prefix >= 8 bytes)
    if data.is_ascii() {
        if constant_time_eq_ignore_ascii_case(data, token_bytes) {
            return true;
        }
        if data.len() >= 8 && data.len() < token_bytes.len() && constant_time_eq_ignore_ascii_case(data, &token_bytes[..data.len()]) {
            return true;
        }
    }

    false
}

/// Linux BLE scanner for discovering paired OmniLink companion beacons via BlueZ.
pub struct BleScanner {
    expected_tokens: Vec<String>,
    adapter_name: Option<String>,
    scan_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl BleScanner {
    /// Creates a new `BleScanner` with no initial expected tokens.
    pub fn new() -> Self {
        Self {
            expected_tokens: Vec::new(),
            adapter_name: None,
            scan_task: Mutex::new(None),
        }
    }

    /// Creates a `BleScanner` configured with an initial expected token.
    pub fn with_token(token: impl Into<String>) -> Self {
        let mut scanner = Self::new();
        scanner.add_token(token);
        scanner
    }

    /// Creates a `BleScanner` configured with multiple expected tokens.
    pub fn with_tokens(tokens: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let mut scanner = Self::new();
        for t in tokens {
            scanner.add_token(t);
        }
        scanner
    }

    /// Adds an expected pairing token to scan for.
    pub fn add_token(&mut self, token: impl Into<String>) {
        let t = token.into();
        let trimmed = t.trim();
        if !trimmed.is_empty() && !self.expected_tokens.iter().any(|tok| tok == trimmed) {
            self.expected_tokens.push(trimmed.to_string());
        }
    }

    /// Returns the currently tracked expected tokens.
    pub fn tokens(&self) -> &[String] {
        &self.expected_tokens
    }

    /// Sets a specific Bluetooth adapter name (e.g., `"hci0"`).
    /// If `None`, the default adapter will be used.
    pub fn set_adapter_name(&mut self, adapter_name: Option<String>) {
        self.adapter_name = adapter_name;
    }

    /// Gets the configured Bluetooth adapter name, if specified.
    pub fn adapter_name(&self) -> Option<&str> {
        self.adapter_name.as_deref()
    }

    /// Checks if a background scanning task is currently active.
    pub fn is_scanning(&self) -> bool {
        if let Ok(lock) = self.scan_task.lock() {
            if let Some(handle) = lock.as_ref() {
                return !handle.is_finished();
            }
        }
        false
    }

    /// Stops the active background scanning task if one is running.
    pub fn stop(&self) {
        if let Ok(mut lock) = self.scan_task.lock() {
            if let Some(handle) = lock.take() {
                handle.abort();
            }
        }
    }

    /// Processes service advertisement data from a device address.
    ///
    /// Returns `Some(BleDiscoveryEvent)` if the device advertises the `OMNILINK_SERVICE_UUID`,
    /// with `token_matched` set according to whether any expected token matched.
    pub fn process_service_data(
        &self,
        device_address: bluer::Address,
        service_data: &HashMap<uuid::Uuid, Vec<u8>>,
    ) -> Option<BleDiscoveryEvent> {
        let payload = service_data.get(&OMNILINK_SERVICE_UUID)?;
        let token_matched = self
            .expected_tokens
            .iter()
            .any(|tok| verify_beacon_payload(payload, tok));

        Some(BleDiscoveryEvent {
            device_address,
            token_matched,
        })
    }

    /// Mock/simulation helper: processes a list of simulated devices and yields discovery events
    /// over an mpsc channel without connecting to BlueZ or requiring Bluetooth hardware.
    pub async fn mock_scan(
        &self,
        devices: Vec<(bluer::Address, HashMap<uuid::Uuid, Vec<u8>>)>,
    ) -> mpsc::Receiver<BleDiscoveryEvent> {
        let (tx, rx) = mpsc::channel(devices.len().max(1));
        for (addr, service_data) in devices {
            if let Some(event) = self.process_service_data(addr, &service_data) {
                let _ = tx.send(event).await;
            }
        }
        rx
    }

    /// Starts scanning for OmniLink proximity beacons using BlueZ via `bluer`.
    ///
    /// Gracefully handles absent or unpowered adapters and unavailable BlueZ daemon.
    /// Returns an `mpsc::Receiver` receiving discovered beacon events.
    pub async fn start_scan(
        &self,
    ) -> Result<mpsc::Receiver<BleDiscoveryEvent>, BleError> {
        self.stop();

        let session = bluer::Session::new().await.map_err(BleError::Bluetooth)?;
        let adapter = match &self.adapter_name {
            Some(name) => session.adapter(name).map_err(|e| {
                if matches!(e.kind, bluer::ErrorKind::NotFound) {
                    BleError::AdapterNotFound(name.clone())
                } else {
                    BleError::Bluetooth(e)
                }
            })?,
            None => session.default_adapter().await.map_err(|e| {
                if matches!(e.kind, bluer::ErrorKind::NotFound) {
                    BleError::NoAdapter
                } else {
                    BleError::Bluetooth(e)
                }
            })?,

        };

        if !adapter.is_powered().await.map_err(BleError::Bluetooth)? {
            tracing::info!("Bluetooth adapter '{}' is powered off; powering on", adapter.name());
            if let Err(e) = adapter.set_powered(true).await {
                tracing::warn!("Failed to power on Bluetooth adapter '{}': {}", adapter.name(), e);
                return Err(BleError::AdapterNotPowered);
            }
        }

        let filter = bluer::DiscoveryFilter {
            transport: bluer::DiscoveryTransport::Le,
            duplicate_data: true,
            ..Default::default()
        };
        if let Err(e) = adapter.set_discovery_filter(filter).await {
            tracing::warn!("Failed to set BLE discovery filter (continuing with default filter): {}", e);
        }

        let mut device_events = adapter
            .discover_devices_with_changes()
            .await
            .map_err(BleError::Bluetooth)?;

        let (tx, rx) = mpsc::channel(64);
        let expected_tokens = self.expected_tokens.clone();

        let handle = tokio::spawn(async move {
            use futures::StreamExt;
            tracing::info!("BLE scanner active on adapter '{}'", adapter.name());

            // 1. Process any devices already cached by BlueZ adapter
            if let Ok(existing_addrs) = adapter.device_addresses().await {
                for addr in existing_addrs {
                    if let Ok(device) = adapter.device(addr) {
                        if let Ok(Some(service_data)) = device.service_data().await {
                            if let Some(payload) = service_data.get(&OMNILINK_SERVICE_UUID) {
                                let token_matched = expected_tokens
                                    .iter()
                                    .any(|tok| verify_beacon_payload(payload, tok));
                                let discovery = BleDiscoveryEvent {
                                    device_address: addr,
                                    token_matched,
                                };
                                tracing::debug!(
                                    "Discovered cached OmniLink beacon at {} (token_matched={})",
                                    addr,
                                    token_matched
                                );
                                if tx.send(discovery).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // 2. Stream incoming discovery events
            while let Some(event) = device_events.next().await {
                if let bluer::AdapterEvent::DeviceAdded(addr) = event {
                    if let Ok(device) = adapter.device(addr) {
                        if let Ok(Some(service_data)) = device.service_data().await {
                            if let Some(payload) = service_data.get(&OMNILINK_SERVICE_UUID) {
                                let token_matched = expected_tokens
                                    .iter()
                                    .any(|tok| verify_beacon_payload(payload, tok));
                                let discovery = BleDiscoveryEvent {
                                    device_address: addr,
                                    token_matched,
                                };
                                tracing::debug!(
                                    "Discovered OmniLink beacon at {} (token_matched={})",
                                    addr,
                                    token_matched
                                );
                                if tx.send(discovery).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            tracing::debug!("BLE scanner task shutting down");
        });

        if let Ok(mut lock) = self.scan_task.lock() {
            if let Some(old_task) = lock.replace(handle) {
                old_task.abort();
            }
        }

        Ok(rx)
    }
}

impl Default for BleScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BleScanner {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_omnilink_service_uuid_parsing_and_constants() {
        let expected_str = "6f6d6e69-6c69-6e6b-0001-000000000000";
        assert_eq!(OMNILINK_SERVICE_UUID.to_string(), expected_str);

        let parsed = uuid::Uuid::parse_str(expected_str).expect("Valid UUID string");
        assert_eq!(parsed, OMNILINK_SERVICE_UUID);

        let expected_bytes: [u8; 16] = [
            0x6f, 0x6d, 0x6e, 0x69, 0x6c, 0x69, 0x6e, 0x6b, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        assert_eq!(*OMNILINK_SERVICE_UUID.as_bytes(), expected_bytes);
    }

    #[test]
    fn test_verify_beacon_payload_utf8_token() {
        let token = "test_ephemeral_token_12345";
        let valid_payload = token.as_bytes();

        assert!(verify_beacon_payload(valid_payload, token));

        // Test with trailing null byte
        let mut with_null = valid_payload.to_vec();
        with_null.push(0x00);
        assert!(verify_beacon_payload(&with_null, token));
    }

    #[test]
    fn test_verify_beacon_payload_hex_raw_token() {
        let hex_token = "0123456789abcdef0123456789abcdef";
        let raw_bytes: [u8; 16] = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ];

        // Raw bytes in payload matching hex string token
        assert!(verify_beacon_payload(&raw_bytes, hex_token));

        // UTF-8 string bytes in payload matching hex string token
        assert!(verify_beacon_payload(hex_token.as_bytes(), hex_token));

        // Case-insensitive matching
        assert!(verify_beacon_payload(
            hex_token.to_uppercase().as_bytes(),
            hex_token
        ));
    }

    #[test]
    fn test_verify_beacon_payload_invalid_token() {
        let expected_token = "valid_token_value";
        let invalid_payload = b"wrong_token_value";
        assert!(!verify_beacon_payload(invalid_payload, expected_token));

        let random_bytes = [0xde, 0xad, 0xbe, 0xef, 0xaa, 0xbb];
        assert!(!verify_beacon_payload(&random_bytes, expected_token));
    }

    #[test]
    fn test_verify_beacon_payload_prefix_and_truncated() {
        let token = "1234567890abcdef1234567890abcdef";
        let full_payload = token.as_bytes();

        // 8-byte prefix (matches MTU legacy beacon)
        assert!(verify_beacon_payload(&full_payload[..8], token));
        assert!(verify_beacon_payload(&full_payload[..16], token));

        // Under 8 bytes is rejected for security against short collisions
        assert!(!verify_beacon_payload(&full_payload[..7], token));
        assert!(!verify_beacon_payload(&full_payload[..4], token));

        // Raw bytes prefix (8 bytes)
        let raw_bytes: [u8; 16] = [
            0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
            0xcd, 0xef,
        ];
        assert!(verify_beacon_payload(&raw_bytes[..8], token));
        assert!(!verify_beacon_payload(&raw_bytes[..7], token));
    }

    #[test]
    fn test_decode_hex_non_ascii_safety() {
        assert_eq!(decode_hex("non_ascii_🦀_emoji"), None);
        assert_eq!(decode_hex("12345"), None); // odd length
        assert_eq!(decode_hex("123g"), None); // non-hex
        assert_eq!(decode_hex("0123456789abcdef"), Some(vec![
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef
        ]));
    }

    #[test]
    fn test_verify_beacon_payload_empty() {
        assert!(!verify_beacon_payload(b"", "non_empty_token"));
        assert!(!verify_beacon_payload(b"some_payload", ""));
        assert!(!verify_beacon_payload(b"", ""));
        assert!(!verify_beacon_payload(b"\0", ""));
        assert!(!verify_beacon_payload(b"\0", "some_token"));
    }

    #[test]
    fn test_ble_scanner_token_management() {
        let mut scanner = BleScanner::new();
        assert!(scanner.tokens().is_empty());
        assert_eq!(scanner.adapter_name(), None);

        scanner.add_token("token_a");
        scanner.add_token("token_b");
        // Duplicate should be ignored
        scanner.add_token("token_a");
        // Empty should be ignored
        scanner.add_token("   ");

        assert_eq!(scanner.tokens().len(), 2);
        assert_eq!(scanner.tokens(), &["token_a", "token_b"]);

        scanner.set_adapter_name(Some("hci0".to_string()));
        assert_eq!(scanner.adapter_name(), Some("hci0"));

        let scanner2 = BleScanner::with_token("single_tok");
        assert_eq!(scanner2.tokens(), &["single_tok"]);

        let scanner3 = BleScanner::with_tokens(vec!["tok1", "tok2"]);
        assert_eq!(scanner3.tokens(), &["tok1", "tok2"]);
    }

    #[test]
    fn test_process_service_data_mocking() {
        let scanner = BleScanner::with_token("secret_pairing_token");
        let dummy_addr = bluer::Address::new([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);

        // Device advertising an unrelated service UUID
        let unrelated_uuid = uuid::uuid!("0000180f-0000-1000-8000-00805f9b34fb"); // Battery service
        let mut map_unrelated = HashMap::new();
        map_unrelated.insert(unrelated_uuid, vec![95]);
        assert_eq!(scanner.process_service_data(dummy_addr, &map_unrelated), None);

        // Device advertising Omnilink UUID with matching token
        let mut map_matched = HashMap::new();
        map_matched.insert(
            OMNILINK_SERVICE_UUID,
            b"secret_pairing_token".to_vec(),
        );
        let res_matched = scanner.process_service_data(dummy_addr, &map_matched);
        assert_eq!(
            res_matched,
            Some(BleDiscoveryEvent {
                device_address: dummy_addr,
                token_matched: true,
            })
        );

        // Device advertising Omnilink UUID with mismatching token
        let mut map_unmatched = HashMap::new();
        map_unmatched.insert(OMNILINK_SERVICE_UUID, b"wrong_token".to_vec());
        let res_unmatched = scanner.process_service_data(dummy_addr, &map_unmatched);
        assert_eq!(
            res_unmatched,
            Some(BleDiscoveryEvent {
                device_address: dummy_addr,
                token_matched: false,
            })
        );
    }

    #[tokio::test]
    async fn test_mock_scan_async_pipeline() {
        let scanner = BleScanner::with_token("my_token");

        let addr1 = bluer::Address::new([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        let addr2 = bluer::Address::new([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
        let addr3 = bluer::Address::new([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);

        let mut data1 = HashMap::new();
        data1.insert(OMNILINK_SERVICE_UUID, b"my_token".to_vec());

        let mut data2 = HashMap::new();
        data2.insert(OMNILINK_SERVICE_UUID, b"unknown_token".to_vec());

        let mut data3 = HashMap::new();
        let other_uuid = uuid::uuid!("00001800-0000-1000-8000-00805f9b34fb");
        data3.insert(other_uuid, vec![1, 2, 3]);

        let devices = vec![(addr1, data1), (addr2, data2), (addr3, data3)];

        let mut rx = scanner.mock_scan(devices).await;

        let ev1 = rx.recv().await.expect("Expected event 1");
        assert_eq!(ev1.device_address, addr1);
        assert!(ev1.token_matched);

        let ev2 = rx.recv().await.expect("Expected event 2");
        assert_eq!(ev2.device_address, addr2);
        assert!(!ev2.token_matched);

        // Device 3 had non-Omnilink UUID, so channel should be closed with no 3rd event
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_ble_scanner_task_lifecycle() {
        let scanner = BleScanner::new();
        assert!(!scanner.is_scanning());
        scanner.stop();
        assert!(!scanner.is_scanning());
    }

    #[tokio::test]
    async fn test_ble_scanner_fallback_nonexistent_adapter() {
        let mut scanner = BleScanner::new();
        scanner.set_adapter_name(Some("nonexistent_hci999".to_string()));

        // Should return an error gracefully without panicking or hanging
        let result = scanner.start_scan().await;
        assert!(result.is_err());
        assert!(!scanner.is_scanning());
    }

    #[tokio::test]
    async fn test_ble_discovery_event_helpers() {
        let addr = bluer::Address::new([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
        let event = BleDiscoveryEvent::new(addr, true);
        assert_eq!(event.device_address, addr);
        assert!(event.token_matched);

        let cloned = event.clone();
        assert_eq!(cloned, event);
    }
}
