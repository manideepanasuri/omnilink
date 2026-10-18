# Omnilink IPC & CLI Specification

This document details the UNIX domain socket IPC interface (`/run/user/<UID>/omnilink.sock`) connecting `omnilink-daemon` to `omnilink-cli`, `omnilink-gui` (Tauri), and Waybar.

---

## 1. Socket Transport & Protocol

- **Socket Path**: `/run/user/<UID>/omnilink.sock` (falls back to `/tmp/omnilink-<UID>.sock`).
- **Framing**: UTF-8 Newline-delimited JSON (`\n`).
- **Pattern**: JSON-RPC 2.0 with asynchronous event streaming.

---

## 2. RPC Methods (Client -> Daemon)

### `status.get`
Returns current pairing and connection telemetry.

**Request**:
```json
{ "id": 1, "method": "status.get", "params": {} }
```

**Response**:
```json
{
  "id": 1,
  "result": {
    "connected": true,
    "device_name": "Galaxy S24 Ultra",
    "battery_pct": 82,
    "is_charging": false,
    "wifi_ssid": "Home-5GHz",
    "cellular_signal": 4,
    "hotspot_active": false
  }
}
```

---

### `device.toggle_hotspot`
Toggles mobile hotspot remotely via Shizuku/ADB on the phone.

**Request**:
```json
{ "id": 2, "method": "device.toggle_hotspot", "params": { "enable": true } }
```

---

### `notification.reply`
Sends an inline reply to an Android notification.

**Request**:
```json
{
  "id": 3,
  "method": "notification.reply",
  "params": {
    "notification_id": "0|com.whatsapp|1001|null|10123",
    "reply_text": "Sounds good, see you at 6!"
  }
}
```

---

### `file.send`
Initiates a high-speed file transfer to `/sdcard/Download/` on the phone.

**Request**:
```json
{
  "id": 4,
  "method": "file.send",
  "params": { "file_path": "/home/manideep/Documents/contract.pdf" }
}
```

---

### `mirror.launch`
Launches low-latency screen or single-app streaming into a floating Hyprland window.

**Request**:
```json
{
  "id": 5,
  "method": "mirror.launch",
  "params": {
    "package_name": "com.spotify.music",
    "floating": true
  }
}
```

---

## 3. Broadcast Events (Daemon -> Clients)

Any connected socket client (Tauri UI, CLI listener) receives real-time JSON events:

```json
{
  "event": "device.status_changed",
  "data": { "battery_pct": 81, "is_charging": true }
}
```

```json
{
  "event": "clipboard.updated",
  "data": { "mime_type": "text/plain", "preview": "https://omarchy.org" }
}
```

```json
{
  "event": "telephony.incoming_call",
  "data": { "caller_name": "Alex", "phone_number": "+15550192834" }
}
```

---

## 4. Waybar Module Integration

The CLI provides a single command outputting Waybar-compatible continuous JSON:

```bash
omnilink status --watch-waybar
```

### Emitted JSON Schema
```json
{
  "text": "82%",
  "tooltip": "Galaxy S24 Ultra\nBattery: 82% (Discharging)\nWi-Fi: Home-5GHz\nSignal: 4/4",
  "class": "connected",
  "percentage": 82
}
```

### Sample `~/.config/waybar/config.jsonc` Module
```jsonc
"custom/omnilink": {
    "exec": "omnilink status --watch-waybar",
    "return-type": "json",
    "format": "󰄡 {text}",
    "format-disconnected": "󰄢 Disconnected",
    "on-click": "omnilink-gui",
    "on-click-right": "omnilink mirror",
    "tooltip": true
}
```
