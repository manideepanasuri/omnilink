# Omnilink Architecture

This document defines the system topology, component responsibilities, and integration points for Omnilink on Linux and Android.

## System Topology

```
+-------------------------------------------------------------------------+
|                              Android Phone                              |
|                                                                         |
|  [Omnilink Android Companion (Kotlin / Compose)]                        |
|    - NotificationListenerService (intercepts & dispatches remote input)|
|    - Foreground Service (QUIC connection client / BLE advertiser)      |
|    - MediaProjection / Shizuku ADB Server (scrcpy video & input pump)   |
|    - ContentProvider query engine (thumbnails & SMS/MMS)                |
+-------------------------------------------------------------------------+
                                    ▲
                                    │ (QUIC over UDP / Noise Protocol)
                                    │ (BLE Proximity Beacon Discovery)
                                    ▼
+-------------------------------------------------------------------------+
|                       Linux Host (Omarchy / Hyprland)                   |
|                                                                         |
|  +-------------------------------------------------------------------+  |
|  | omnilink-daemon (Rust / Tokio)                                    |  |
|  |   - QUIC Endpoint listener & session manager                      |  |
|  |   - BlueZ BLE scanner & PipeWire HFP audio routing                |  |
|  |   - Wayland clipboard listener (ext-data-control)                 |  |
|  |   - D-Bus client for desktop notifications (Quickshell / swaync)  |  |
|  |   - Embedded scrcpy video decoder & input injection bridge        |  |
|  |   - Local UNIX socket IPC server (/run/user/$UID/omnilink.sock)   |  |
|  +-------------------------------------------------------------------+  |
|           ▲                                            ▲                |
|           │ UNIX IPC                                   │ UNIX IPC       |
|           ▼                                            ▼                |
|  +-----------------------------------+   +---------------------------+  |
|  | omnilink-gui (Tauri v2)           |   | omnilink-cli & Status     |  |
|  |   - Svelte/Tailwind Dashboard     |   |   - Waybar/Quickshell JSON|  |
|  |   - Photos, SMS, App Drawer       |   |   - Hotkey triggers       |  |
|  +-----------------------------------+   +---------------------------+  |
+-------------------------------------------------------------------------+
```

## 1. Linux Daemon (`omnilink-daemon`)

The daemon is the headless brain of Omnilink. It runs as a `systemd` user service:

- **Lifecycle**: Starts automatically on login (`systemd --user enable --now omnilink`).
- **IPC Interface**: Exposes a local UNIX domain socket at `/run/user/<UID>/omnilink.sock` using JSON-RPC / MessagePack for CLI and Tauri communication.
- **D-Bus Integration**:
  - Connects to `org.freedesktop.Notifications` via `zbus` to trigger native desktop alerts and register action callbacks (for inline replies).
  - Connects to `org.bluez` for BLE device discovery and Bluetooth HFP handsfree profile negotiation.
- **Audio Routing**:
  - Interacts directly with PipeWire (`pipewire-rs`) to route incoming call audio from the phone's Bluetooth HFP sink to the user's active microphone and headset without manual patching.
- **Wayland Integration**:
  - Connects to the Wayland compositor via `wl-clipboard` or `wlr-data-control` / `ext-data-control` protocols to monitor selection changes and push foreign clipboard selections.

## 2. Android Companion (`omnilink-android`)

The mobile companion runs on Android 12+ (optimized for Samsung One UI).

- **Execution Model**:
  - Foreground Service with a persistent, low-power notification to ensure the OS never kills the socket process.
  - BLE Advertiser: Periodically broadcasts an encrypted beacon so the Linux desktop detects when the phone is physically nearby.
- **Privilege Layers**:
  - **Tier 1 (Base - Public APIs)**:
    - `NotificationListenerService`: Reads incoming notifications, passes them to Linux, and executes `RemoteInput` actions when user replies from desktop.
    - `ClipboardManager`: Reads and writes local system clipboard events.
    - MediaStore query: Fast cursor-based indexing for recent camera photos.
  - **Tier 2 (Elevated - Shizuku / Wireless ADB)**:
    - Spawns the `scrcpy-server.jar` without user confirmation prompts.
    - Grants instant access to secondary virtual display creation (enables launching single apps into individual desktop windows without mirroring the main phone display).
    - Toggles mobile hotspot programmatically without manual permission popups.

## 3. Desktop Application (`omnilink-gui`)

Built with Tauri v2 for near-zero memory footprint (~40MB RAM) and hardware-accelerated UI rendering.

- **Frontend**: Svelte 5 + Tailwind CSS.
- **Panels**:
  - **Overview**: Phone battery, cellular signal, storage, quick toggles (Do Not Disturb, Hotspot, Find Phone).
  - **Photos**: Masonry grid showing local cached thumbnails of the last 1,000 photos. Clicking opens high-res; dragging out drops the file directly into other Linux apps.
  - **Messages**: Threaded conversation view for SMS/RCS with real-time send/receive.
  - **App Drawer**: Lists installed phone apps with icons. Clicking an app launches that specific app in a dedicated floating Hyprland window.
- **Window Management**:
  - Configured with `class = "omnilink-gui"` for easy Hyprland window rule matching.

## 4. Hyprland Integration Rules

Omnilink is built first-class for tiling window managers. The recommended `hyprland.conf` rules:

```ini
# Floating window for single-app streaming and phone mirror
windowrulev2 = float, class:^(omnilink-mirror)$
windowrulev2 = size 420 860, class:^(omnilink-mirror)$
windowrulev2 = center, class:^(omnilink-mirror)$

# Floating Omnilink dashboard
windowrulev2 = float, class:^(omnilink-gui)$
windowrulev2 = size 1050 700, class:^(omnilink-gui)$
windowrulev2 = center, class:^(omnilink-gui)$
```
