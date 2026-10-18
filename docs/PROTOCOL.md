# Omnilink Network Protocol Specification

This document details the transport layer, cryptography, pairing handshake, and wire packet schemas used between the Linux host and the Android companion.

## 1. Transport & Connectivity

Omnilink communicates over local Wi-Fi and Bluetooth LE.

- **Discovery Layer**:
  - **Primary**: Bluetooth LE (BLE) advertisement. Phone advertises UUID `6f6d-6e69-6c69-6e6b-0001` with an ephemeral encrypted token.
  - **Secondary**: Local UDP multicast/mDNS service `_omnilink._udp.local.` on port 42424.
- **Transport Layer**:
  - **Protocol**: QUIC over UDP (implemented via `quinn` in Rust).
  - **Why QUIC**: Multiplexed independent streams prevent head-of-line blocking (a heavy 50MB video file transfer will never delay an instant clipboard sync or notification alert). Fast 0-RTT reconnects when waking from sleep.

## 2. Pairing & Cryptography

All communications are end-to-end encrypted with mutual authentication.

1. **Key Generation**: Both phone and Linux daemon generate an Ed25519 identity keypair upon initial setup.
2. **Onboarding Handshake (QR Code)**:
   - Linux GUI displays a QR code containing:
     ```json
     {
       "v": 1,
       "name": "omarchy-workstation",
       "pk": "<base64_ed25519_public_key>",
       "addrs": ["192.168.1.100:42424", "[fe80::...]:42424"],
       "token": "<one_time_pairing_secret>"
     }
     ```
   - Android scans the QR code, verifies the fingerprint, and sends its public key signed with the one-time pairing secret.
3. **Session Encryption**:
   - Noise Protocol Handshake (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) or mutual TLS 1.3 with ed25519 certificates.
   - Symmetric session cipher: ChaCha20-Poly1305 with rotating re-keying every 1GB or 1 hour.

## 3. Multiplexed Stream Allocation

Inside a single QUIC connection, Omnilink maintains dedicated streams:

| Stream ID / Type | Purpose | Encoding |
| :--- | :--- | :--- |
| **Stream 0 (Uni/Bi)** | Control, Telemetry & Heartbeat | MessagePack / JSON |
| **Stream 1 (Bi)** | Bidirectional Clipboard Sync | Binary Header + Payload |
| **Stream 2 (Bi)** | Notifications & Action Replies | MessagePack / JSON |
| **Stream 3 (Bi)** | High-Throughput File & Photo Sync | Chunked Binary Stream |
| **Stream 4 (Uni)** | Screen & App Mirroring Video Stream | Raw H.264 / H.265 NAL units |
| **Stream 5 (Uni)** | Remote Input Events (Mouse / Touch / Keys)| Compact Binary Structs |

## 4. Packet Schemas (Stream 0 & 2)

### Control Message (Stream 0)

```rust
pub enum ControlPacket {
    Ping { timestamp: u64 },
    Pong { timestamp: u64 },
    DeviceStatus {
        battery_pct: u8,
        is_charging: bool,
        wifi_ssid: Option<String>,
        cellular_signal: u8, // 0..4 bars
        hotspot_active: bool,
    },
    Command {
        action: CommandAction, // ToggleHotspot, RingPhone, LaunchApp { package: String }
    },
}
```

### Clipboard Packet (Stream 1)

```rust
pub struct ClipboardPacket {
    pub sequence_id: u64,
    pub mime_type: String,      // "text/plain;charset=utf-8" or "image/png"
    pub is_sensitive: bool,     // True if originated from a password manager
    pub payload: Vec<u8>,
}
```

### Notification Packet (Stream 2)

```rust
pub struct NotificationPacket {
    pub id: String,             // Unique key on Android (sbn.key)
    pub package_name: String,   // "org.telegram.messenger", "com.whatsapp"
    pub app_name: String,       // "Telegram"
    pub title: String,          // "Sender Name"
    pub text: String,           // "Message preview"
    pub icon_png: Option<Vec<u8>>,
    pub timestamp: i64,
    pub can_reply: bool,        // True if RemoteInput is supported
    pub actions: Vec<NotificationAction>, // e.g., ["Mark as read", "Reply"]
}

pub struct NotificationReplyPacket {
    pub notification_id: String,
    pub action_key: String,
    pub reply_text: String,
}
```
