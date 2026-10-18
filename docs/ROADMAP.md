# Omnilink Implementation Roadmap & Milestones

This document defines the 4-phase rollout strategy and acceptance criteria for Omnilink.

---

## Milestone 1: Core Foundation & Daily Essentials
*Goal: Establish rock-solid pairing, instantaneous clipboard sync, and two-way notifications.*

- [ ] Scaffold Rust workspace (`omnilink-protocol`, `omnilink-daemon`, `omnilink-cli`) and Android project.
- [ ] Implement Ed25519 pairing handshake via terminal QR code / BLE advertisement.
- [ ] Establish multiplexed QUIC connection with keep-alive heartbeat.
- [ ] Bidirectional Wayland clipboard sync (text & images) with loop detection.
- [ ] Android `NotificationListenerService` forwarding alerts to desktop D-Bus (`org.freedesktop.Notifications`).
- [ ] Inline reply callback from desktop notification bubbles back to Android.
- [ ] Two-way notification dismissal sync.

**Acceptance Criteria**:
1. Copying text on phone pastes instantly on Linux under Hyprland, and vice versa.
2. Incoming SMS or WhatsApp notification pops up on Omarchy desktop; typing a reply and hitting Enter sends the reply through the phone.

---

## Milestone 2: Desktop GUI & Media Transfer
*Goal: Build the visual dashboard, photo gallery, and fast drag-and-drop file transfers.*

- [ ] Initialize Tauri v2 application with Svelte 5 and Tailwind CSS.
- [ ] Connect Tauri frontend to `omnilink-daemon` over UNIX domain socket.
- [ ] Implement thumbnail caching engine for Android camera roll.
- [ ] Build responsive masonry photo gallery with zoom and drag-to-desktop capability.
- [ ] Implement high-speed chunked file transfer (`omnilink send <file>` and desktop drag-drop).
- [ ] Export status endpoints for Waybar / Quickshell (battery percentage, connection state, Wi-Fi).

**Acceptance Criteria**:
1. Opening the desktop app immediately shows cached photo thumbnails from the phone.
2. Dragging a photo out of Omnilink onto GIMP, Discord, or a file manager saves the full-res image.
3. Waybar displays real-time phone battery and connection icon.

---

## Milestone 3: Embedded Screen Mirroring & App Streaming
*Goal: Low-latency phone mirroring and isolated single-app streaming into Hyprland windows.*

- [ ] Integrate embedded `scrcpy-server` launcher via wireless ADB / Shizuku.
- [ ] Implement hardware-accelerated H.264 video decoding (VA-API/NVDEC) into a Wayland surface.
- [ ] Map mouse clicks, touch swipes, and keyboard typing from Linux into Android input injection.
- [ ] Implement secondary virtual display manager to launch specific apps without mirror clutter.
- [ ] Add Hyprland window rules for auto-floating and sizing mirror windows.

**Acceptance Criteria**:
1. Clicking an app icon in Omnilink opens that specific Android app in a sleek floating window under 40ms latency.
2. Interacting with the streamed window does not lock up or interrupt the physical phone's screen.

---

## Milestone 4: Telephony & Audio Routing
*Goal: Answer and make phone calls directly using Linux PC microphone and speakers.*

- [ ] Configure BlueZ HFP (Hands-Free Profile) daemon endpoints.
- [ ] Wire PipeWire audio streams to route phone call audio to active desktop headset/mic.
- [ ] Build desktop incoming call banner with "Answer", "Decline", and "Decline with SMS".
- [ ] Implement in-app dialer pad to initiate phone calls from desktop.

**Acceptance Criteria**:
1. An incoming call rings on Linux desktop; clicking Answer lets you talk through your desktop microphone and hear caller audio clearly.
