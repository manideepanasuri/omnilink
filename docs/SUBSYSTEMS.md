# Omnilink Subsystem Specifications

This document defines the implementation specifications for each core feature subsystem within Omnilink.

---

## 1. Clipboard Synchronization Subsystem

### Wayland Integration
- Omnilink utilizes Wayland's `ext-data-control-v1` (with fallback to `wlr-data-control-unstable-v1`).
- Listens to data source selections without requiring an active focused window.

### Loop Prevention
- When copying content from the phone to Linux, Omnilink computes a BLAKE3 hash of the payload and records it in a `recent_hashes` circular buffer.
- When the Linux Wayland clipboard fires a selection event, Omnilink computes the hash: if it matches the latest buffer entry, transmission is skipped.

### Supported MIME Types
- Plain Text: `text/plain;charset=utf-8`, `text/plain`.
- Images: `image/png`, `image/jpeg`.

### Sensitive Data Concealment
- On Android: Honors `ClipDescription.EXTRA_IS_SENSITIVE` (introduced in Android 13). Sensitive clips are not synced or are cleared after a 60-second TTL.
- On Linux: Honors `x-kde-passwordManagerHint` or secret-service markers.

---

## 2. Notification & Quick Reply Subsystem

### Flow Diagram
```
[Android Notification Event]
           │
           ▼
[NotificationListenerService] ──(QUIC Stream 2)──▶ [omnilink-daemon]
                                                           │
                                                           ▼
                                                [D-Bus: org.freedesktop.Notifications]
                                                           │
                                                           ▼
                                                [Quickshell / Desktop Popup]
                                                           │ (User types inline reply)
                                                           ▼
[Android executes RemoteInput] ◀──(QUIC Stream 2)── [omnilink-daemon Action Callback]
```

### Two-Way Dismissal
- When a notification is closed on the desktop, a `NotificationDismissed { id }` packet is sent to Android, triggering `cancelNotification(id)`.
- When cleared on the phone, Android broadcasts a dismiss packet, and the Linux daemon invokes `CloseNotification` on D-Bus.

### App Blacklist
- Stored locally in `~/.config/omnilink/config.toml` under `[notifications.blacklist]`.
- Silences noisy system notifications (e.g. charging state, Google Play Services background tasks).

---

## 3. Photo Gallery & File Transfer Subsystem

### Virtual Photo Gallery
- Android indexes the device's camera roll (`MediaStore.Images.Media.EXTERNAL_CONTENT_URI`).
- On initial connection, Android streams compressed WebP thumbnails (256x256 px) for the 500 most recent photos.
- Thumbnails are stored in `~/.cache/omnilink/thumbnails/<photo_id>.webp`.
- Full-resolution photos are fetched on demand only when clicked or dragged into a Linux application.

### High-Speed File Transfer
- Dragging any file onto the Tauri UI or running `omnilink send <path>` sends chunks over QUIC Stream 3.
- Target directory on phone: `/sdcard/Download/`.
- File integrity verified via BLAKE3 checksum post-transfer.

---

## 4. Screen Mirroring & App Streaming Subsystem

### Embedded `scrcpy` Engine
- Omnilink embeds the optimized `scrcpy-server.jar`.
- Connected via local TCP socket or Shizuku/ADB wireless connection.
- Phone's hardware `MediaCodec` encodes the display to H.264 at 60fps (bitrate: 8–16 Mbps).

### Hardware Video Decoding
- On Linux, the H.264 stream is decoded via hardware acceleration (VA-API on Intel/AMD, NVDEC on NVIDIA) using FFmpeg / GStreamer bindings in Rust.
- Video frames are rendered directly into a Wayland surface.

### Single-App Isolation (Virtual Display)
- Leveraging Android 11+ hidden APIs (accessible via Shizuku / ADB):
  1. Omnilink creates a secondary virtual display on the phone.
  2. Launches the target application onto the virtual display (`am start --display <id> <package>`).
  3. Streams only the virtual display to Linux, allowing the user to use other apps on their physical phone simultaneously.

---

## 5. Calls & Audio Subsystem

### Bluetooth HFP & PipeWire Routing
- Android restricts third-party apps from capturing phone call cellular audio over software sockets.
- Omnilink leverages Bluetooth Hands-Free Profile (HFP/HSP):
  1. Linux desktop registers as an HFP Audio Gateway / Handsfree Unit via BlueZ (`org.bluez`).
  2. PipeWire automatically maps the incoming call audio to the default system audio sink (headphones) and source (microphone).
- Omnilink provides the UI overlay:
  - Caller ID popup on incoming call with "Answer", "Decline", and "Decline with SMS".
  - In-app dialer pad to place outbound calls through the phone.
