//! Packet definitions, wire formats, and serialization codecs for Omnilink streams.

use serde::{Deserialize, Serialize};

use crate::error::{ProtocolError, Result};

/// Generic helper to serialize any serializable value to MessagePack bytes.
///
/// Uses named fields (maps) for cross-language compatibility with companion clients.
pub fn to_msgpack<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec_named(value).map_err(Into::into)
}

/// Generic helper to deserialize any deserializable value from MessagePack bytes.
pub fn from_msgpack<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    rmp_serde::from_slice(bytes).map_err(Into::into)
}

/// Generic helper to serialize any serializable value to a JSON string.
pub fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| ProtocolError::Serialization(e.to_string()))
}

/// Generic helper to deserialize any deserializable value from a JSON string.
pub fn from_json<'a, T: Deserialize<'a>>(json_str: &'a str) -> Result<T> {
    serde_json::from_str(json_str).map_err(|e| ProtocolError::Deserialization(e.to_string()))
}

/// Legacy packet enumeration for basic ping-pong heartbeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Packet {
    /// Ping packet with unix timestamp in milliseconds.
    Ping { timestamp: u64 },
    /// Pong packet with echoed unix timestamp in milliseconds.
    Pong { timestamp: u64 },
}

/// Actions executable on remote Android companion devices via Stream 0 command packets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandAction {
    /// Toggle Wi-Fi hotspot state on the phone.
    ToggleHotspot,
    /// Ring the phone at max volume for device locating.
    RingPhone,
    /// Launch an installed application package on the phone.
    LaunchApp { package: String },
}

/// Control, telemetry, and heartbeat messages multiplexed over Stream 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlPacket {
    /// Ping message containing unix timestamp in milliseconds.
    Ping { timestamp: u64 },
    /// Pong response containing echoed unix timestamp in milliseconds.
    Pong { timestamp: u64 },
    /// Periodic or event-driven device telemetry sent by the Android companion.
    DeviceStatus {
        /// Battery percentage (0..100).
        battery_pct: u8,
        /// Charging status.
        is_charging: bool,
        /// Connected Wi-Fi SSID, if connected.
        wifi_ssid: Option<String>,
        /// Cellular signal strength in bars (0..4).
        cellular_signal: u8,
        /// Mobile hotspot active state.
        hotspot_active: bool,
    },
    /// Command sent from Linux host to Android companion.
    Command { action: CommandAction },
}

impl ControlPacket {
    /// Serialize this `ControlPacket` to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Deserialize a `ControlPacket` from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }

    /// Serialize this `ControlPacket` to a JSON string.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Deserialize a `ControlPacket` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        from_json(json_str)
    }
}

impl From<crate::types::DeviceStatus> for ControlPacket {
    fn from(status: crate::types::DeviceStatus) -> Self {
        ControlPacket::DeviceStatus {
            battery_pct: status.battery_pct,
            is_charging: status.is_charging,
            wifi_ssid: status.wifi_ssid,
            cellular_signal: status.cellular_signal,
            hotspot_active: status.hotspot_active,
        }
    }
}

impl TryFrom<ControlPacket> for crate::types::DeviceStatus {
    type Error = ProtocolError;

    fn try_from(packet: ControlPacket) -> Result<Self> {
        match packet {
            ControlPacket::DeviceStatus {
                battery_pct,
                is_charging,
                wifi_ssid,
                cellular_signal,
                hotspot_active,
            } => Ok(crate::types::DeviceStatus {
                battery_pct,
                is_charging,
                wifi_ssid,
                cellular_signal,
                hotspot_active,
            }),
            _ => Err(ProtocolError::InvalidHeader(
                "ControlPacket variant is not DeviceStatus".to_string(),
            )),
        }
    }
}

/// Bidirectional clipboard synchronization packet framed over Stream 1.
///
/// Binary wire header format:
/// `[sequence_id: 8 bytes BE, is_sensitive: 1 byte, mime_type_len: 2 bytes BE, mime_type: N bytes, payload: M bytes]`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardPacket {
    /// Monotonically increasing sequence ID to avoid echo loops and out-of-order writes.
    pub sequence_id: u64,
    /// MIME type (e.g. "text/plain;charset=utf-8" or "image/png").
    pub mime_type: String,
    /// Flag indicating whether the clipboard content originated from a sensitive source (e.g. password manager).
    pub is_sensitive: bool,
    /// Raw clipboard data payload.
    pub payload: Vec<u8>,
}

impl ClipboardPacket {
    /// Minimum binary header size: 8 bytes (seq) + 1 byte (flags) + 2 bytes (mime_len) = 11 bytes.
    pub const FIXED_HEADER_LEN: usize = 11;

    /// Encodes the packet into the Stream 1 binary wire format:
    /// `[sequence_id: 8 bytes BE, is_sensitive: 1 byte, mime_type_len: 2 bytes BE, mime_type: N bytes, payload: M bytes]`
    pub fn encode_binary(&self) -> Result<Vec<u8>> {
        let mime_bytes = self.mime_type.as_bytes();
        let mime_len = u16::try_from(mime_bytes.len()).map_err(|_| {
            ProtocolError::Serialization(
                "MIME type exceeds maximum length of 65535 bytes".to_string(),
            )
        })?;

        let mut buf =
            Vec::with_capacity(Self::FIXED_HEADER_LEN + mime_bytes.len() + self.payload.len());
        buf.extend_from_slice(&self.sequence_id.to_be_bytes());
        buf.push(if self.is_sensitive { 1 } else { 0 });
        buf.extend_from_slice(&mime_len.to_be_bytes());
        buf.extend_from_slice(mime_bytes);
        buf.extend_from_slice(&self.payload);
        Ok(buf)
    }

    /// Decodes a packet from the Stream 1 binary wire format.
    pub fn decode_binary(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < Self::FIXED_HEADER_LEN {
            return Err(ProtocolError::InvalidHeader(format!(
                "Insufficient bytes for clipboard header: expected at least {}, got {}",
                Self::FIXED_HEADER_LEN,
                bytes.len()
            )));
        }

        let sequence_id = u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);

        let is_sensitive = bytes[8] != 0;

        let mime_type_len = u16::from_be_bytes([bytes[9], bytes[10]]) as usize;

        let mime_end = Self::FIXED_HEADER_LEN + mime_type_len;
        if bytes.len() < mime_end {
            return Err(ProtocolError::InvalidHeader(format!(
                "Insufficient bytes for MIME type: expected at least {mime_end}, got {}",
                bytes.len()
            )));
        }

        let mime_type = std::str::from_utf8(&bytes[Self::FIXED_HEADER_LEN..mime_end])?.to_string();

        let payload = bytes[mime_end..].to_vec();

        Ok(Self {
            sequence_id,
            mime_type,
            is_sensitive,
            payload,
        })
    }

    /// Serialize this `ClipboardPacket` to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Deserialize a `ClipboardPacket` from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }

    /// Serialize this `ClipboardPacket` to a JSON string.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Deserialize a `ClipboardPacket` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        from_json(json_str)
    }
}

/// Helper function to encode a `ClipboardPacket` into binary wire format.
pub fn encode_clipboard_binary(packet: &ClipboardPacket) -> Result<Vec<u8>> {
    packet.encode_binary()
}

/// Helper function to decode a `ClipboardPacket` from binary wire format.
pub fn decode_clipboard_binary(bytes: &[u8]) -> Result<ClipboardPacket> {
    ClipboardPacket::decode_binary(bytes)
}

/// Action item associated with an incoming notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    /// Unique action identifier key.
    pub key: String,
    /// Display title for the action button (e.g. "Reply", "Mark as read").
    pub title: String,
    /// Indicates whether the action accepts inline text input (e.g. RemoteInput).
    pub is_input: bool,
}

/// Incoming notification packet received over Stream 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPacket {
    /// Unique Android notification key (sbn.key).
    pub id: String,
    /// Application package name (e.g. "org.telegram.messenger").
    pub package_name: String,
    /// Human-readable application name (e.g. "Telegram").
    pub app_name: String,
    /// Notification title / sender name.
    pub title: String,
    /// Notification body text preview.
    pub text: String,
    /// App icon or large icon encoded as PNG bytes, if available.
    pub icon_png: Option<Vec<u8>>,
    /// Notification creation timestamp in milliseconds.
    pub timestamp: i64,
    /// True if the notification supports inline reply.
    pub can_reply: bool,
    /// Available action buttons for this notification.
    pub actions: Vec<NotificationAction>,
}

impl NotificationPacket {
    /// Serialize this `NotificationPacket` to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Deserialize a `NotificationPacket` from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }

    /// Serialize this `NotificationPacket` to a JSON string.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Deserialize a `NotificationPacket` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        from_json(json_str)
    }
}

/// Outgoing reply packet for an actionable notification transmitted over Stream 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationReplyPacket {
    /// Target notification unique key.
    pub notification_id: String,
    /// Key of the target action being invoked.
    pub action_key: String,
    /// Body text of the reply.
    pub reply_text: String,
}

impl NotificationReplyPacket {
    /// Serialize this `NotificationReplyPacket` to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Deserialize a `NotificationReplyPacket` from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }

    /// Serialize this `NotificationReplyPacket` to a JSON string.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Deserialize a `NotificationReplyPacket` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        from_json(json_str)
    }
}

/// Bidirectional dismissal synchronization packet for Stream 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationDismissPacket {
    /// Target notification unique key.
    pub id: String,
}

impl NotificationDismissPacket {
    /// Serialize this `NotificationDismissPacket` to MessagePack bytes.
    pub fn to_msgpack(&self) -> Result<Vec<u8>> {
        to_msgpack(self)
    }

    /// Deserialize a `NotificationDismissPacket` from MessagePack bytes.
    pub fn from_msgpack(bytes: &[u8]) -> Result<Self> {
        from_msgpack(bytes)
    }

    /// Serialize this `NotificationDismissPacket` to a JSON string.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Deserialize a `NotificationDismissPacket` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self> {
        from_json(json_str)
    }
}

/// Multiplexed packet types transmitted over QUIC Stream 2.
///
/// Binary frame payload format: `[type_id: 1 byte][payload: N bytes UTF-8 JSON]`
/// - `0x01`: Posted notification (`NotificationPacket`)
/// - `0x02`: Quick reply (`NotificationReplyPacket`)
/// - `0x03`: Dismissal (`NotificationDismissPacket`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationStreamPacket {
    /// Mobile notification event posted to desktop.
    Posted(NotificationPacket),
    /// Desktop quick reply event forwarded to Android RemoteInput.
    Reply(NotificationReplyPacket),
    /// Dismissal event (desktop closed or Android swipe-dismissed).
    Dismiss(NotificationDismissPacket),
}

impl NotificationStreamPacket {
    pub const TYPE_POSTED: u8 = 1;
    pub const TYPE_REPLY: u8 = 2;
    pub const TYPE_DISMISS: u8 = 3;

    /// Encodes this stream packet into wire format: `[type_id: 1B][json_bytes: NB]`.
    pub fn encode_binary(&self) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        match self {
            Self::Posted(packet) => {
                buf.push(Self::TYPE_POSTED);
                buf.extend_from_slice(packet.to_json()?.as_bytes());
            }
            Self::Reply(packet) => {
                buf.push(Self::TYPE_REPLY);
                buf.extend_from_slice(packet.to_json()?.as_bytes());
            }
            Self::Dismiss(packet) => {
                buf.push(Self::TYPE_DISMISS);
                buf.extend_from_slice(packet.to_json()?.as_bytes());
            }
        }
        Ok(buf)
    }

    /// Decodes a stream packet from wire format: `[type_id: 1B][json_bytes: NB]`.
    pub fn decode_binary(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(ProtocolError::InvalidHeader(
                "Empty payload for NotificationStreamPacket".to_string(),
            ));
        }

        let type_id = bytes[0];
        let json_slice = std::str::from_utf8(&bytes[1..])?;

        match type_id {
            Self::TYPE_POSTED => {
                let packet: NotificationPacket = serde_json::from_str(json_slice)
                    .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;
                Ok(Self::Posted(packet))
            }
            Self::TYPE_REPLY => {
                let packet: NotificationReplyPacket = serde_json::from_str(json_slice)
                    .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;
                Ok(Self::Reply(packet))
            }
            Self::TYPE_DISMISS => {
                let packet: NotificationDismissPacket = serde_json::from_str(json_slice)
                    .map_err(|e| ProtocolError::Deserialization(e.to_string()))?;
                Ok(Self::Dismiss(packet))
            }
            other => Err(ProtocolError::InvalidHeader(format!(
                "Unknown Stream 2 packet type ID: {other}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy_packet_ping_pong_serialization() {
        let ping = Packet::Ping {
            timestamp: 1729250000000,
        };
        let json = serde_json::to_string(&ping).expect("Failed to serialize Ping packet");
        let parsed: Packet =
            serde_json::from_str(&json).expect("Failed to deserialize Ping packet");

        assert_eq!(ping, parsed);
    }

    #[test]
    fn test_control_packet_ping_pong_msgpack_and_json() {
        let ping = ControlPacket::Ping {
            timestamp: 1729250000000,
        };

        // MessagePack roundtrip
        let msgpack_bytes = ping
            .to_msgpack()
            .expect("Failed to serialize Ping to msgpack");
        let parsed_msgpack = ControlPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize Ping from msgpack");
        assert_eq!(ping, parsed_msgpack);

        // JSON roundtrip
        let json_str = ping.to_json().expect("Failed to serialize Ping to JSON");
        let parsed_json =
            ControlPacket::from_json(&json_str).expect("Failed to deserialize Ping from JSON");
        assert_eq!(ping, parsed_json);

        let pong = ControlPacket::Pong {
            timestamp: 1729250000100,
        };

        let msgpack_bytes = pong
            .to_msgpack()
            .expect("Failed to serialize Pong to msgpack");
        let parsed_msgpack = ControlPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize Pong from msgpack");
        assert_eq!(pong, parsed_msgpack);

        let json_str = pong.to_json().expect("Failed to serialize Pong to JSON");
        let parsed_json =
            ControlPacket::from_json(&json_str).expect("Failed to deserialize Pong from JSON");
        assert_eq!(pong, parsed_json);
    }

    #[test]
    fn test_control_packet_device_status_msgpack_and_json() {
        let status_with_wifi = ControlPacket::DeviceStatus {
            battery_pct: 85,
            is_charging: true,
            wifi_ssid: Some("Home-5GHz".to_string()),
            cellular_signal: 4,
            hotspot_active: false,
        };

        // MessagePack
        let msgpack_bytes = status_with_wifi
            .to_msgpack()
            .expect("Failed to serialize DeviceStatus to msgpack");
        let parsed_msgpack = ControlPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize DeviceStatus from msgpack");
        assert_eq!(status_with_wifi, parsed_msgpack);

        // JSON
        let json_str = status_with_wifi
            .to_json()
            .expect("Failed to serialize DeviceStatus to JSON");
        let parsed_json = ControlPacket::from_json(&json_str)
            .expect("Failed to deserialize DeviceStatus from JSON");
        assert_eq!(status_with_wifi, parsed_json);

        // None wifi_ssid and active hotspot
        let status_no_wifi = ControlPacket::DeviceStatus {
            battery_pct: 15,
            is_charging: false,
            wifi_ssid: None,
            cellular_signal: 1,
            hotspot_active: true,
        };

        let msgpack_bytes = status_no_wifi
            .to_msgpack()
            .expect("Failed to serialize DeviceStatus to msgpack");
        let parsed_msgpack = ControlPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize DeviceStatus from msgpack");
        assert_eq!(status_no_wifi, parsed_msgpack);

        let json_str = status_no_wifi
            .to_json()
            .expect("Failed to serialize DeviceStatus to JSON");
        let parsed_json = ControlPacket::from_json(&json_str)
            .expect("Failed to deserialize DeviceStatus from JSON");
        assert_eq!(status_no_wifi, parsed_json);
    }

    #[test]
    fn test_control_packet_command_msgpack_and_json() {
        let commands = vec![
            ControlPacket::Command {
                action: CommandAction::ToggleHotspot,
            },
            ControlPacket::Command {
                action: CommandAction::RingPhone,
            },
            ControlPacket::Command {
                action: CommandAction::LaunchApp {
                    package: "org.telegram.messenger".to_string(),
                },
            },
        ];

        for cmd in commands {
            let msgpack_bytes = cmd
                .to_msgpack()
                .expect("Failed to serialize Command to msgpack");
            let parsed_msgpack = ControlPacket::from_msgpack(&msgpack_bytes)
                .expect("Failed to deserialize Command from msgpack");
            assert_eq!(cmd, parsed_msgpack);

            let json_str = cmd.to_json().expect("Failed to serialize Command to JSON");
            let parsed_json = ControlPacket::from_json(&json_str)
                .expect("Failed to deserialize Command from JSON");
            assert_eq!(cmd, parsed_json);
        }
    }

    #[test]
    fn test_clipboard_packet_binary_roundtrip() {
        // 1. Standard text payload
        let packet = ClipboardPacket {
            sequence_id: 42,
            mime_type: "text/plain;charset=utf-8".to_string(),
            is_sensitive: false,
            payload: b"Hello, Wayland!".to_vec(),
        };

        let encoded = packet.encode_binary().expect("Failed to encode binary");
        let decoded = ClipboardPacket::decode_binary(&encoded).expect("Failed to decode binary");
        assert_eq!(packet, decoded);

        // 2. Sensitive payload
        let sensitive_packet = ClipboardPacket {
            sequence_id: 1001,
            mime_type: "text/plain".to_string(),
            is_sensitive: true,
            payload: b"super_secret_master_password".to_vec(),
        };

        let encoded = encode_clipboard_binary(&sensitive_packet).expect("Failed to encode binary");
        let decoded = decode_clipboard_binary(&encoded).expect("Failed to decode binary");
        assert_eq!(sensitive_packet, decoded);
        assert!(decoded.is_sensitive);

        // 3. Empty payload
        let empty_packet = ClipboardPacket {
            sequence_id: 1002,
            mime_type: "text/plain".to_string(),
            is_sensitive: false,
            payload: Vec::new(),
        };

        let encoded = empty_packet
            .encode_binary()
            .expect("Failed to encode binary");
        let decoded = ClipboardPacket::decode_binary(&encoded).expect("Failed to decode binary");
        assert_eq!(empty_packet, decoded);
        assert!(decoded.payload.is_empty());

        // 4. Binary payload (PNG image header bytes)
        let binary_payload = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF, 0xFE,
        ];
        let image_packet = ClipboardPacket {
            sequence_id: 1003,
            mime_type: "image/png".to_string(),
            is_sensitive: false,
            payload: binary_payload,
        };

        let encoded = image_packet
            .encode_binary()
            .expect("Failed to encode binary");
        let decoded = ClipboardPacket::decode_binary(&encoded).expect("Failed to decode binary");
        assert_eq!(image_packet, decoded);
    }

    #[test]
    fn test_clipboard_packet_binary_error_handling() {
        // Less than fixed header length (11 bytes)
        let too_short = vec![0u8; 10];
        let result = ClipboardPacket::decode_binary(&too_short);
        assert!(matches!(result, Err(ProtocolError::InvalidHeader(_))));

        // Header claims mime_type_len = 10, but only 5 bytes remain
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&1u64.to_be_bytes()); // 8 bytes
        truncated.push(0); // 1 byte
        truncated.extend_from_slice(&10u16.to_be_bytes()); // 2 bytes -> claims mime_len = 10
        truncated.extend_from_slice(b"abcde"); // only 5 bytes
        let result = ClipboardPacket::decode_binary(&truncated);
        assert!(matches!(result, Err(ProtocolError::InvalidHeader(_))));

        // Invalid UTF-8 in mime_type
        let mut invalid_utf8 = Vec::new();
        invalid_utf8.extend_from_slice(&1u64.to_be_bytes());
        invalid_utf8.push(0);
        invalid_utf8.extend_from_slice(&2u16.to_be_bytes());
        invalid_utf8.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        invalid_utf8.extend_from_slice(b"payload");
        let result = ClipboardPacket::decode_binary(&invalid_utf8);
        assert!(matches!(result, Err(ProtocolError::Deserialization(_))));
    }

    #[test]
    fn test_notification_packet_msgpack_and_json() {
        let notification = NotificationPacket {
            id: "0|org.telegram.messenger|12345|tag".to_string(),
            package_name: "org.telegram.messenger".to_string(),
            app_name: "Telegram".to_string(),
            title: "Pavel Durov".to_string(),
            text: "Welcome to Omnilink!".to_string(),
            icon_png: Some(vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A]),
            timestamp: 1729255000000,
            can_reply: true,
            actions: vec![
                NotificationAction {
                    key: "reply_action_key".to_string(),
                    title: "Reply".to_string(),
                    is_input: true,
                },
                NotificationAction {
                    key: "mark_read".to_string(),
                    title: "Mark as Read".to_string(),
                    is_input: false,
                },
            ],
        };

        // MessagePack roundtrip
        let msgpack_bytes = notification
            .to_msgpack()
            .expect("Failed to serialize NotificationPacket to msgpack");
        let parsed_msgpack = NotificationPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize NotificationPacket from msgpack");
        assert_eq!(notification, parsed_msgpack);

        // JSON roundtrip
        let json_str = notification
            .to_json()
            .expect("Failed to serialize NotificationPacket to JSON");
        let parsed_json = NotificationPacket::from_json(&json_str)
            .expect("Failed to deserialize NotificationPacket from JSON");
        assert_eq!(notification, parsed_json);

        // Without icon and with empty actions
        let simple_notification = NotificationPacket {
            id: "1|com.example.app|1".to_string(),
            package_name: "com.example.app".to_string(),
            app_name: "Example".to_string(),
            title: "Test".to_string(),
            text: "Simple text".to_string(),
            icon_png: None,
            timestamp: 1729256000000,
            can_reply: false,
            actions: Vec::new(),
        };

        let msgpack_bytes = simple_notification
            .to_msgpack()
            .expect("Failed to serialize simple NotificationPacket to msgpack");
        let parsed_msgpack = NotificationPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize simple NotificationPacket from msgpack");
        assert_eq!(simple_notification, parsed_msgpack);
    }

    #[test]
    fn test_notification_reply_packet_msgpack_and_json() {
        let reply = NotificationReplyPacket {
            notification_id: "0|org.telegram.messenger|12345|tag".to_string(),
            action_key: "reply_action_key".to_string(),
            reply_text: "Replying directly from Linux Hyprland!".to_string(),
        };

        // MessagePack roundtrip
        let msgpack_bytes = reply
            .to_msgpack()
            .expect("Failed to serialize NotificationReplyPacket to msgpack");
        let parsed_msgpack = NotificationReplyPacket::from_msgpack(&msgpack_bytes)
            .expect("Failed to deserialize NotificationReplyPacket from msgpack");
        assert_eq!(reply, parsed_msgpack);

        // JSON roundtrip
        let json_str = reply
            .to_json()
            .expect("Failed to serialize NotificationReplyPacket to JSON");
        let parsed_json = NotificationReplyPacket::from_json(&json_str)
            .expect("Failed to deserialize NotificationReplyPacket from JSON");
        assert_eq!(reply, parsed_json);
    }

    #[test]
    fn test_notification_dismiss_packet_msgpack_and_json() {
        let dismiss = NotificationDismissPacket {
            id: "0|org.telegram.messenger|12345".to_string(),
        };

        let msgpack = dismiss.to_msgpack().expect("to_msgpack");
        let parsed_msgpack = NotificationDismissPacket::from_msgpack(&msgpack).expect("from_msgpack");
        assert_eq!(dismiss, parsed_msgpack);

        let json = dismiss.to_json().expect("to_json");
        let parsed_json = NotificationDismissPacket::from_json(&json).expect("from_json");
        assert_eq!(dismiss, parsed_json);
    }

    #[test]
    fn test_notification_stream_packet_binary_roundtrip() {
        let posted = NotificationStreamPacket::Posted(NotificationPacket {
            id: "notif-1".to_string(),
            package_name: "org.example".to_string(),
            app_name: "Example".to_string(),
            title: "Hello".to_string(),
            text: "World".to_string(),
            icon_png: None,
            timestamp: 1000,
            can_reply: true,
            actions: vec![],
        });

        let encoded_posted = posted.encode_binary().expect("encode posted");
        let decoded_posted = NotificationStreamPacket::decode_binary(&encoded_posted).expect("decode posted");
        assert_eq!(posted, decoded_posted);

        let reply = NotificationStreamPacket::Reply(NotificationReplyPacket {
            notification_id: "notif-1".to_string(),
            action_key: "inline-reply".to_string(),
            reply_text: "Replying back!".to_string(),
        });

        let encoded_reply = reply.encode_binary().expect("encode reply");
        let decoded_reply = NotificationStreamPacket::decode_binary(&encoded_reply).expect("decode reply");
        assert_eq!(reply, decoded_reply);

        let dismiss = NotificationStreamPacket::Dismiss(NotificationDismissPacket {
            id: "notif-1".to_string(),
        });

        let encoded_dismiss = dismiss.encode_binary().expect("encode dismiss");
        let decoded_dismiss = NotificationStreamPacket::decode_binary(&encoded_dismiss).expect("decode dismiss");
        assert_eq!(dismiss, decoded_dismiss);

        // Unknown type error handling
        let bad_type = vec![0x99, b'{', b'}'];
        assert!(NotificationStreamPacket::decode_binary(&bad_type).is_err());

        // Empty error handling
        assert!(NotificationStreamPacket::decode_binary(&[]).is_err());
    }

    #[test]
    fn test_device_status_conversion() {
        let status = crate::types::DeviceStatus {
            battery_pct: 90,
            is_charging: true,
            wifi_ssid: Some("Office-WiFi".to_string()),
            cellular_signal: 3,
            hotspot_active: false,
        };

        let packet: ControlPacket = status.clone().into();
        let converted_back: crate::types::DeviceStatus = packet
            .try_into()
            .expect("Conversion back to DeviceStatus failed");

        assert_eq!(status, converted_back);

        let ping = ControlPacket::Ping { timestamp: 1234 };
        let bad_conv: Result<crate::types::DeviceStatus> = ping.try_into();
        assert!(matches!(bad_conv, Err(ProtocolError::InvalidHeader(_))));
    }
}
