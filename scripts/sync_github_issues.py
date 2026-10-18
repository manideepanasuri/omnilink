#!/usr/bin/env python3
import subprocess
import sys
import time

GH_BIN = "/home/manideep/.local/share/mise/installs/gh/latest/gh_2.102.0_linux_amd64/bin/gh"
REPO = "manideepanasuri/omnilink"

def run_cmd(args):
    print(f"Running: {' '.join(args[:4])} ...")
    res = subprocess.run(args, capture_output=True, text=True)
    if res.returncode != 0:
        print(f"ERROR: {res.stderr}")
        return False, res.stderr
    print(f"SUCCESS: {res.stdout.strip()}")
    return True, res.stdout.strip()

ISSUES_TO_UPDATE = [
    {
        "num": 1,
        "title": "Issue 1A: [Rust] Cargo Workspace & Crates Scaffolding",
        "labels": "milestone-1,subsystem:protocol",
        "body": """## Objective & Scope
Initialize the root Rust Cargo workspace and scaffold empty crate manifests.

## Architecture & References
- Reference: `README.md` (Directory Layout)
- Reference: `docs/ARCHITECTURE.md` Section 1
- Crates to scaffold:
  - `crates/omnilink-protocol`: Shared serialization types
  - `crates/omnilink-daemon`: Linux background service
  - `crates/omnilink-cli`: Command-line interface and status provider

## Technical Requirements & Constraints
- Root `Cargo.toml` using `[workspace]` with members `crates/*`.
- Rust edition 2021.
- Setup shared workspace dependencies (tokio, serde, thiserror, anyhow).
- Enforce strict workspace lints (`clippy::all`, `warn(missing_docs)` optional).

## Acceptance Criteria
- [ ] Root `Cargo.toml` defines workspace containing `omnilink-protocol`, `omnilink-daemon`, `omnilink-cli`.
- [ ] `cargo check --workspace` completes with 0 errors and 0 warnings.
- [ ] `cargo test --workspace` passes cleanly.

## Dependencies & Blockers
- None. First task in repository.

## Out of Scope
- Implementing network loops, D-Bus handlers, or Android code (handled in separate issues).

## Verification & Testing
```bash
cargo check --workspace
cargo test --workspace
```"""
    },
    {
        "num": 2,
        "title": "Issue 2A: [Protocol] Core Wire Packet Types & Serialization",
        "labels": "milestone-1,subsystem:protocol",
        "body": """## Objective & Scope
Implement shared wire protocol packet schemas and serialization routines in `crates/omnilink-protocol`.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 3 & 4
- Implements: `ControlPacket`, `ClipboardPacket`, `NotificationPacket`, `NotificationReplyPacket`

## Technical Requirements & Constraints
- Implement `serde::Serialize` and `serde::Deserialize` for all packet enums and structs.
- MessagePack serialization (`rmp-serde`) for Stream 0 and Stream 2 control/notification payloads.
- Binary header format with sequence ID and MIME type for Stream 1 clipboard payloads.
- Unit tests validating roundtrip encode/decode integrity and backward-compatible enum tagging.

## Acceptance Criteria
- [ ] `ControlPacket` enum implemented with Ping, Pong, DeviceStatus, Command variants.
- [ ] `ClipboardPacket` struct implemented with sequence_id, mime_type, is_sensitive, payload.
- [ ] `NotificationPacket` and `NotificationReplyPacket` structs implemented.
- [ ] Comprehensive unit tests verify roundtrip serialization without data loss.

## Dependencies & Blockers
- Prerequisite: Issue 1A.

## Out of Scope
- Network socket listeners or QUIC endpoint initialization (handled in Issue 2D).

## Verification & Testing
```bash
cargo test -p omnilink-protocol
```"""
    },
    {
        "num": 3,
        "title": "Issue 3A: [Daemon] Wayland Data-Control Listener & BLAKE3 Echo Filter",
        "labels": "milestone-1,subsystem:wayland",
        "body": """## Objective & Scope
Implement Wayland selection listener and clipboard loop prevention in `crates/omnilink-daemon`.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 1
- Reference: `docs/DAEMON_IMPLEMENTATION.md` Section 3
- Implements: `ClipboardFilter` and `wayland-client` data control loop.

## Technical Requirements & Constraints
- Connect via `ext-data-control-v1` (with fallback to `wlr-data-control-unstable-v1`).
- Monitor primary and selection clipboards without requiring active window focus.
- Circular buffer `recent_hashes` of size 16 storing BLAKE3 hashes to prevent infinite echo loops.
- Support `text/plain;charset=utf-8` and `image/png` MIME types.

## Acceptance Criteria
- [ ] Daemon detects selection change when user copies text or image on Hyprland/Wayland.
- [ ] Hashing filter correctly drops copy events for data whose BLAKE3 hash is in recent history.
- [ ] Emits internal `ClipboardEvent` to tokio mpsc channel.

## Dependencies & Blockers
- Prerequisite: Issue 1A, Issue 2A.

## Out of Scope
- Android clipboard handling (Issue 3B) or QUIC transmission (Issue 3C).

## Verification & Testing
```bash
cargo test -p omnilink-daemon clipboard
```"""
    },
    {
        "num": 4,
        "title": "Issue 4A: [Android] NotificationListenerService & Packet Dispatcher",
        "labels": "milestone-1,subsystem:android",
        "body": """## Objective & Scope
Implement Android `NotificationListenerService` extracting incoming notifications and RemoteInput action keys.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 2
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 3
- Implements: `OmnilinkNotificationService.kt`

## Technical Requirements & Constraints
- Android API 31+ (Android 12 to 15).
- Listen to `onNotificationPosted` and inspect `StatusBarNotification`.
- Extract title, text, postTime, app package, and find actions containing `RemoteInput`.
- Filter out local blacklist packages (e.g. Google Play services background sync).
- Serialize into `NotificationPostedPacket`.

## Acceptance Criteria
- [ ] Posting test notification triggers `onNotificationPosted`.
- [ ] Correctly identifies if notification supports inline reply (`canReply = true`).
- [ ] Dispatches packet to internal service queue.

## Dependencies & Blockers
- Prerequisite: Issue 1B.

## Out of Scope
- D-Bus integration on Linux (Issue 4B) or remote reply callback execution (Issue 4C).

## Verification & Testing
```bash
./gradlew :app:testDebugUnitTest
```"""
    },
    {
        "num": 5,
        "title": "Issue 5A: [Daemon] UNIX Domain Socket JSON-RPC Server",
        "labels": "milestone-2,subsystem:gui",
        "body": """## Objective & Scope
Implement the local UNIX domain socket IPC server in `omnilink-daemon` for CLI and Tauri communication.

## Architecture & References
- Reference: `docs/IPC_SPECIFICATION.md` Section 1, 2, 3
- Reference: `docs/DAEMON_IMPLEMENTATION.md` Section 5
- Socket Path: `/run/user/<UID>/omnilink.sock` (fallback `/tmp/omnilink-<UID>.sock`)

## Technical Requirements & Constraints
- Asynchronous framing via newline-delimited JSON (`\\n`).
- JSON-RPC 2.0 request/response handling (`status.get`, `device.toggle_hotspot`).
- Broadcast event streaming channel for connected subscribers (`device.status_changed`, `clipboard.updated`).
- Tokio UnixListener with graceful socket cleanup on shutdown.

## Acceptance Criteria
- [ ] Socket binds successfully at `/run/user/<UID>/omnilink.sock`.
- [ ] Querying `{\"id\":1,\"method\":\"status.get\",\"params\":{}}` returns valid JSON status response.
- [ ] Disconnecting clients does not crash daemon.

## Dependencies & Blockers
- Prerequisite: Issue 1A, Issue 2A.

## Out of Scope
- Tauri frontend UI (Issue 5B, 5C) or Waybar CLI module (Issue 7).

## Verification & Testing
```bash
cargo test -p omnilink-daemon ipc
```"""
    },
    {
        "num": 6,
        "title": "Issue 6A: [Daemon/CLI] High-Speed Chunked File Transfer (Stream 3)",
        "labels": "milestone-2,subsystem:protocol",
        "body": """## Objective & Scope
Implement chunked binary file transfer over QUIC Stream 3 with BLAKE3 checksum verification.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 3
- Reference: `docs/SUBSYSTEMS.md` Section 3
- Implements: `omnilink send <file_path>` CLI command and daemon sender/receiver.

## Technical Requirements & Constraints
- Dedicated multiplexed QUIC Stream 3.
- Framing: [Total Size (u64)] [BLAKE3 Hash (32 bytes)] [Filename Len (u16)] [Filename] [64KB Chunks...].
- Backpressure handling and progress callback reporting.
- Target destination directory on Android: `/sdcard/Download/`.

## Acceptance Criteria
- [ ] Transferring a 100MB file finishes without buffer exhaustion.
- [ ] Receiver validates computed BLAKE3 hash against header hash before finalizing file.
- [ ] CLI shows terminal progress bar with transfer rate (MB/s).

## Dependencies & Blockers
- Prerequisite: Issue 2D, Issue 2E.

## Out of Scope
- Photo gallery thumbnail queries (Issue 6B) or GUI drag-and-drop (Issue 6C).

## Verification & Testing
```bash
cargo test -p omnilink-daemon file_transfer
```"""
    },
    {
        "num": 7,
        "title": "Issue 7: [CLI/Wayland] Waybar Streaming JSON Status & Hyprland Rules",
        "labels": "milestone-2,subsystem:wayland",
        "body": """## Objective & Scope
Build the Waybar integration status provider in `omnilink-cli` and provide reference Hyprland window rules.

## Architecture & References
- Reference: `docs/IPC_SPECIFICATION.md` Section 4
- Reference: `docs/ARCHITECTURE.md` Section 4
- Command: `omnilink status --watch-waybar`

## Technical Requirements & Constraints
- Connect to daemon UNIX socket `/run/user/<UID>/omnilink.sock`.
- Emit continuous newline-delimited Waybar JSON:
  `{\"text\":\"82%\",\"tooltip\":\"...\",\"class\":\"connected\",\"percentage\":82}`.
- Auto-reconnect with exponential backoff if daemon restarts.
- Document and package `hyprland.conf` window rules for `class:^(omnilink-gui)$` and `class:^(omnilink-mirror)$`.

## Acceptance Criteria
- [ ] `omnilink status --watch-waybar` produces compliant Waybar JSON stream.
- [ ] Hyprland windowrulev2 configuration floats and sizes mirror/GUI windows properly.

## Dependencies & Blockers
- Prerequisite: Issue 5A.

## Out of Scope
- Tauri GUI implementation (Issue 5B) or screen mirroring engine (Issue 8B).

## Verification & Testing
```bash
cargo run -p omnilink-cli -- status --watch-waybar
```"""
    },
    {
        "num": 8,
        "title": "Issue 8A: [Android] Shizuku Scrcpy-Server Launcher & Stream Provider",
        "labels": "milestone-3,subsystem:android",
        "body": """## Objective & Scope
Package and execute `scrcpy-server.jar` on Android via Shizuku without requiring a USB connection.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 4
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 4
- Spawns: `com.genymobile.scrcpy.Server` 2.4 via Shizuku privileged process.

## Technical Requirements & Constraints
- Require Shizuku permission check (`Shizuku.checkSelfPermission()`).
- Copy `scrcpy-server.jar` to internal storage and invoke via `app_process`.
- Server configuration: `port=42425`, `audio=false`, `control=true`, `max_fps=60`, `bit_rate=8000000`.
- Provide TCP listening socket or route NAL units into QUIC Stream 4.

## Acceptance Criteria
- [ ] Server launches cleanly under Shizuku without crashing or prompting adb dialogs.
- [ ] Stream client connecting to port 42425 receives valid H.264 video SPS/PPS headers.

## Dependencies & Blockers
- Prerequisite: Issue 1B.

## Out of Scope
- Linux hardware video decoding (Issue 8B) or input injection handling (Issue 8C).

## Verification & Testing
```bash
./gradlew :app:assembleDebug
```"""
    },
    {
        "num": 9,
        "title": "Issue 9A: [Android] Shizuku Secondary Virtual Display Controller",
        "labels": "milestone-3,subsystem:android",
        "body": """## Objective & Scope
Implement Android secondary virtual display manager using Shizuku privileged APIs for single-app streaming.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 4
- Reference: `docs/ARCHITECTURE.md` Section 2
- Command: `am start-activity --display <display_id> <package>`

## Technical Requirements & Constraints
- Utilize hidden `DisplayManager.createVirtualDisplay` with privileged flags via reflection/Shizuku.
- Launch target app directly onto the secondary virtual display.
- Ensure the phone's primary display remains interactive for other tasks.

## Acceptance Criteria
- [ ] Creating virtual display returns valid non-zero display ID.
- [ ] Launching an app onto the display renders frames without stealing primary phone screen focus.

## Dependencies & Blockers
- Prerequisite: Issue 8A.

## Out of Scope
- Linux window mapping or Tauri app drawer UI (Issue 9B).

## Verification & Testing
```bash
./gradlew :app:testDebugUnitTest
```"""
    },
    {
        "num": 10,
        "title": "Issue 10A: [Daemon] BlueZ HFP Registration & PipeWire SCO Node Linker",
        "labels": "milestone-4,subsystem:wayland",
        "body": """## Objective & Scope
Register Bluetooth Hands-Free Profile (HFP) via BlueZ and route phone call audio through PipeWire.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 5
- Reference: `docs/ARCHITECTURE.md` Section 1
- Crates: `bluer`, `pipewire` (or D-Bus BlueZ profile manager)

## Technical Requirements & Constraints
- Register HFP/HSP Handsfree unit profile on `org.bluez`.
- Detect when SCO audio socket connects upon active phone call.
- Link PipeWire audio source/sink nodes to route mobile audio to PC headset and microphone.

## Acceptance Criteria
- [ ] Desktop registers as valid Handsfree Unit under BlueZ.
- [ ] SCO audio stream establishes bidirectional link with default PipeWire sink and source.

## Dependencies & Blockers
- Prerequisite: Issue 1A.

## Out of Scope
- In-app GUI dialer (Issue 10C) or call state IPC broadcasting (Issue 10B).

## Verification & Testing
```bash
cargo test -p omnilink-daemon telephony
```"""
    }
]

NEW_ISSUES_TO_CREATE = [
    {
        "title": "Issue 1B: [Android] Companion Project Scaffolding (Gradle KTS & Compose)",
        "labels": "milestone-1,subsystem:android",
        "body": """## Objective & Scope
Scaffold the native Android companion project in `android/` using Kotlin, Jetpack Compose, and Gradle KTS.

## Architecture & References
- Reference: `README.md` (Directory Layout)
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 1
- Dependencies: AndroidX Lifecycle, Jetpack Compose Material3, Coroutines, Tink Crypto, Shizuku.

## Technical Requirements & Constraints
- Min SDK: 31 (Android 12), Target SDK: 35.
- Kotlin 2.0+ with Compose Compiler Gradle plugin.
- Setup `OmnilinkApplication` and placeholder `MainActivity` with basic status card.

## Acceptance Criteria
- [ ] `./gradlew assembleDebug` compiles cleanly and produces `app-debug.apk`.
- [ ] APK installs and runs on Android 12+ device/emulator without runtime crash.

## Dependencies & Blockers
- None. Parallel with Issue 1A.

## Out of Scope
- Network protocols or background services (handled in Issue 2C, 2E).

## Verification & Testing
```bash
cd android && ./gradlew assembleDebug
```"""
    },
    {
        "title": "Issue 1C: [CI] GitHub Actions Workflow Matrix for Rust & Android",
        "labels": "milestone-1",
        "body": """## Objective & Scope
Create GitHub Actions CI workflow in `.github/workflows/ci.yml` validating all pull requests.

## Technical Requirements & Constraints
- Rust matrix job: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`.
- Android job: Setup JDK 17, cache Gradle dependencies, run `./gradlew assembleDebug testDebugUnitTest`.
- Fast execution with caching for Cargo target and Gradle wrapper.

## Acceptance Criteria
- [ ] PR validation runs automatically on push to main or pull request.
- [ ] Both Rust and Android jobs report status checks to GitHub.

## Dependencies & Blockers
- Prerequisite: Issue 1A, Issue 1B.

## Out of Scope
- Release packaging, CD, or deployment.

## Verification & Testing
- Trigger manual workflow run on GitHub Actions."""
    },
    {
        "title": "Issue 2B: [Daemon] Ed25519 Keystore & Terminal QR Pairing Generator",
        "labels": "milestone-1,subsystem:protocol",
        "body": """## Objective & Scope
Generate, persist Ed25519 identity keypair on Linux and render terminal ASCII QR code for onboarding.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 2
- Key path: `~/.local/share/omnilink/identity.key` (mode 0600)
- Command: `omnilink pair`

## Technical Requirements & Constraints
- Generate keypair using `ed25519-dalek` with OS secure randomness (`getrandom`).
- JSON handshake payload containing: protocol version, hostname, base64 public key, local IP addresses (port 42424), and one-time secret token.
- Render compact QR code to terminal stdout via `qrcode` crate.

## Acceptance Criteria
- [ ] Keypair persists across daemon restarts with proper file permissions.
- [ ] `omnilink pair` displays readable terminal QR code matching protocol schema.

## Dependencies & Blockers
- Prerequisite: Issue 1A, Issue 2A.

## Out of Scope
- Android scanner (Issue 2C) or QUIC session setup (Issue 2D).

## Verification & Testing
```bash
cargo run -p omnilink-cli -- pair
```"""
    },
    {
        "title": "Issue 2C: [Android] ML Kit QR Scanner & Identity Keystore",
        "labels": "milestone-1,subsystem:android",
        "body": """## Objective & Scope
Implement pairing screen in Android companion with ML Kit camera QR code scanner and secure key storage.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 2
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 1
- Library: `com.google.mlkit:barcode-scanning`

## Technical Requirements & Constraints
- CameraX preview with ML Kit barcode analyzer.
- Parse JSON pairing payload and validate schema.
- Generate and store Android device Ed25519 keypair in Android KeyStore / EncryptedSharedPreferences.
- Store paired desktop public key and network endpoints.

## Acceptance Criteria
- [ ] Camera scans pairing QR code in under 1 second.
- [ ] Successfully deserializes pairing token and stores keys securely.

## Dependencies & Blockers
- Prerequisite: Issue 1B.

## Out of Scope
- QUIC socket transmission (handled in Issue 2E).

## Verification & Testing
```bash
./gradlew :app:assembleDebug
```"""
    },
    {
        "title": "Issue 2D: [Daemon] QUIC Endpoint Listener & Stream Multiplexer",
        "labels": "milestone-1,subsystem:protocol",
        "body": """## Objective & Scope
Implement multiplexed QUIC endpoint listener and Stream 0 heartbeat loop in `omnilink-daemon`.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 1 & 3
- Reference: `docs/DAEMON_IMPLEMENTATION.md` Section 2
- Crate: `quinn` listening on UDP `0.0.0.0:42424`

## Technical Requirements & Constraints
- Mutual TLS 1.3 with self-signed Ed25519 certificates matching paired public keys.
- Stream 0: Periodic Ping/Pong heartbeats and `DeviceStatus` telemetry updates.
- Maintain connection state in `DaemonState.session`.

## Acceptance Criteria
- [ ] QUIC listener binds to port 42424 and accepts authenticated client.
- [ ] Unauthenticated clients or unknown certificates are immediately rejected.
- [ ] Heartbeat detects dropped connection within 5 seconds.

## Dependencies & Blockers
- Prerequisite: Issue 2A, Issue 2B.

## Out of Scope
- Clipboard, notification, or file streaming logic (handled in respective stream issues).

## Verification & Testing
```bash
cargo test -p omnilink-daemon quic
```"""
    },
    {
        "title": "Issue 2E: [Android] QUIC Client Connection & Heartbeat Loop",
        "labels": "milestone-1,subsystem:android",
        "body": """## Objective & Scope
Implement Android background client connection to Linux QUIC endpoint with sticky foreground service.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 1 & 3
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 2
- Service: `OmnilinkSyncService` running as `FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE`.

## Technical Requirements & Constraints
- Connect to desktop IP and port 42424 using paired Ed25519 credentials.
- Handle One UI background optimization prompts (`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`).
- Stream 0: Respond to Ping packets with Pong and send battery/Wi-Fi telemetry changes.

## Acceptance Criteria
- [ ] Service maintains active QUIC session when phone screen is locked.
- [ ] Automatically reconnects when device transitions between Wi-Fi and mobile data.

## Dependencies & Blockers
- Prerequisite: Issue 2C, Issue 2D.

## Out of Scope
- Content syncing (clipboard, notifications, files).

## Verification & Testing
```bash
./gradlew :app:assembleDebug
```"""
    },
    {
        "title": "Issue 2F: [Discovery] BLE Proximity Advertisement & Scanner",
        "labels": "milestone-1,subsystem:protocol",
        "body": """## Objective & Scope
Implement BLE proximity beacon advertising on Android and scanning on Linux for seamless auto-connect.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 1
- Service UUID: `6f6d-6e69-6c69-6e6b-0001`
- Linux Crate: `bluer`

## Technical Requirements & Constraints
- Android: Broadcast BLE advertising packet with paired ephemeral token when screen turns on or Wi-Fi changes.
- Linux Daemon: BlueZ LE scanner detects beacon and triggers QUIC connection check without manual user interaction.

## Acceptance Criteria
- [ ] Walking in Bluetooth range of PC automatically prompts or executes background session reconnection.
- [ ] Low battery impact on Android device (<1% per day for BLE advertising).

## Dependencies & Blockers
- Prerequisite: Issue 2D, Issue 2E.

## Out of Scope
- Audio profiles or HFP telephony (handled in Issue 10A).

## Verification & Testing
```bash
cargo test -p omnilink-daemon ble
```"""
    },
    {
        "title": "Issue 3B: [Android] Clipboard Listener, Sensitive Filter & Injector",
        "labels": "milestone-1,subsystem:android",
        "body": """## Objective & Scope
Implement Android `ClipboardManager` listener and clipboard injection in the Android companion app.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 1
- Component: `OmnilinkClipboardManager.kt`

## Technical Requirements & Constraints
- Register `ClipboardManager.OnPrimaryClipChangedListener`.
- Respect `ClipDescription.EXTRA_IS_SENSITIVE` (Android 13+): Do not synchronize sensitive/password manager clips.
- Support `text/plain` and `image/png` clip types.
- Provide injection method to safely write incoming text/image to system clipboard without recursive re-broadcast.

## Acceptance Criteria
- [ ] Copying plain text or image on Android fires clipboard change event.
- [ ] Sensitive clips are ignored.
- [ ] Injected clips successfully populate system clipboard.

## Dependencies & Blockers
- Prerequisite: Issue 1B.

## Out of Scope
- QUIC transport (Issue 3C) or Linux Wayland data control (Issue 3A).

## Verification & Testing
```bash
./gradlew :app:testDebugUnitTest
```"""
    },
    {
        "title": "Issue 3C: [Integration] Clipboard QUIC Stream 1 Transport Sync",
        "labels": "milestone-1,subsystem:wayland,subsystem:android",
        "body": """## Objective & Scope
Wire end-to-end clipboard synchronization over dedicated QUIC Stream 1 between Linux daemon and Android.

## Architecture & References
- Reference: `docs/PROTOCOL.md` Section 3
- Stream: Dedicated bidirectional Stream 1.

## Technical Requirements & Constraints
- Stream 1 pump in `omnilink-daemon` serializing `ClipboardPacket`.
- Stream 1 receiver on Android injecting to `ClipboardManager`.
- Sub-50ms latency over local Wi-Fi.

## Acceptance Criteria
- [ ] Copying text on Android enables immediate `wl-paste` on Linux under Hyprland.
- [ ] Copying on Linux immediately updates Android clipboard without infinite feedback loops.

## Dependencies & Blockers
- Prerequisite: Issue 2D, Issue 2E, Issue 3A, Issue 3B.

## Out of Scope
- Desktop GUI or status bar indicators.

## Verification & Testing
- Manual verification copying between Linux terminal and phone clipboard."""
    },
    {
        "title": "Issue 4B: [Daemon] D-Bus Desktop Notification Forwarder via zbus",
        "labels": "milestone-1,subsystem:wayland",
        "body": """## Objective & Scope
Forward incoming Android notifications to the Linux desktop via D-Bus `org.freedesktop.Notifications`.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 2
- Reference: `docs/DAEMON_IMPLEMENTATION.md` Section 4
- Crate: `zbus` using `FreedesktopNotifications` proxy.

## Technical Requirements & Constraints
- Forward app name, summary, body, and icon bytes.
- If `can_reply == true`, register `"inline-reply"` action button.
- Map returned notification ID to remote Android notification key.

## Acceptance Criteria
- [ ] Incoming `NotificationPostedPacket` produces native desktop popup on Hyprland/Omarchy (swaync/dunst).
- [ ] Shows app name and message body accurately.

## Dependencies & Blockers
- Prerequisite: Issue 2A, Issue 2D.

## Out of Scope
- Processing inline reply input (Issue 4C) or dismissal sync (Issue 4D).

## Verification & Testing
```bash
cargo test -p omnilink-daemon notifications
```"""
    },
    {
        "title": "Issue 4C: [Integration] Desktop Inline Reply to Android RemoteInput Bridge",
        "labels": "milestone-1,subsystem:wayland,subsystem:android",
        "body": """## Objective & Scope
Implement end-to-end inline reply execution from Linux notification action callbacks back to Android chat apps.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 2
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 3
- Protocol: `NotificationReplyPacket` over Stream 2.

## Technical Requirements & Constraints
- Linux: Catch D-Bus `ActionInvoked` or `NotificationReplied` signal and send reply packet over Stream 2.
- Android: Match notification key, locate `RemoteInput`, apply `RemoteInput.addResultsToIntent`, and dispatch `actionIntent.send()`.

## Acceptance Criteria
- [ ] Typing a reply into desktop notification popup sends reply through WhatsApp/Telegram/SMS on phone.
- [ ] No UI popup or interruption required on phone display.

## Dependencies & Blockers
- Prerequisite: Issue 4A, Issue 4B.

## Out of Scope
- SMS conversation thread history (handled in GUI).

## Verification & Testing
- End-to-end test replying to test message notification."""
    },
    {
        "title": "Issue 4D: [Integration] Bidirectional Notification Dismissal Synchronization",
        "labels": "milestone-1,subsystem:wayland,subsystem:android",
        "body": """## Objective & Scope
Synchronize notification dismissals between Linux desktop notifications and Android notification bar.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 2
- APIs: D-Bus `NotificationClosed` / `CloseNotification` and Android `cancelNotification(key)`.

## Technical Requirements & Constraints
- When dismissed on Linux: Send `NotificationDismissed` packet to trigger `cancelNotification`.
- When cleared on Android: Trigger D-Bus `CloseNotification` on Linux to remove stale popup.

## Acceptance Criteria
- [ ] Dismissing notification on desktop immediately clears it from phone status bar.
- [ ] Swiping away notification on phone immediately dismisses desktop popup.

## Dependencies & Blockers
- Prerequisite: Issue 4A, Issue 4B.

## Out of Scope
- App notification blacklist settings UI.

## Verification & Testing
- Test dismissing notifications from desktop and device."""
    },
    {
        "title": "Issue 5B: [GUI] Tauri v2 Project Scaffold & IPC Socket Client Bridge",
        "labels": "milestone-2,subsystem:gui",
        "body": """## Objective & Scope
Initialize the Tauri v2 desktop application in `crates/omnilink-gui` with Svelte 5 and connect to daemon IPC.

## Architecture & References
- Reference: `README.md` (Directory Layout)
- Reference: `docs/ARCHITECTURE.md` Section 3
- Tech: Tauri v2, Rust backend, Svelte 5 frontend, Tailwind CSS.

## Technical Requirements & Constraints
- Tauri backend connects to `/run/user/<UID>/omnilink.sock` via local UNIX domain stream.
- Expose Tauri invoke commands for RPC methods (`status.get`, `device.toggle_hotspot`).
- Forward daemon broadcast events to Svelte store (`event: device.status_changed`).

## Acceptance Criteria
- [ ] `cargo tauri dev` launches application window.
- [ ] UI establishes live socket connection to running daemon.
- [ ] Idle memory consumption is verified under 45MB RAM.

## Dependencies & Blockers
- Prerequisite: Issue 5A.

## Out of Scope
- Specific feature tabs (Photos, App drawer, Dialer).

## Verification & Testing
```bash
cargo check -p omnilink-gui
```"""
    },
    {
        "title": "Issue 5C: [GUI] Device Overview Dashboard & Quick Toggles UI",
        "labels": "milestone-2,subsystem:gui",
        "body": """## Objective & Scope
Build the primary Overview dashboard tab in Tauri GUI displaying phone status and quick action toggles.

## Architecture & References
- Reference: `docs/ARCHITECTURE.md` Section 3
- Aesthetic: Omarchy / Hyprland dark mode aesthetic.

## Technical Requirements & Constraints
- Components: Battery indicator with charging icon, Wi-Fi SSID, cellular signal bars (0-4).
- Quick toggles: Mobile Hotspot, Find My Phone (ring device), Do Not Disturb.
- Real-time updates driven by incoming `device.status_changed` IPC events.

## Acceptance Criteria
- [ ] Battery level and network state update in real-time when changed on phone.
- [ ] Clicking Hotspot toggle triggers RPC `device.toggle_hotspot`.

## Dependencies & Blockers
- Prerequisite: Issue 5B.

## Out of Scope
- Photo gallery (Issue 6C) or screen mirror viewer (Issue 8B).

## Verification & Testing
- Launch UI and verify component responsiveness with mock IPC server."""
    },
    {
        "title": "Issue 6B: [Android] MediaStore Thumbnail Pipeline & WebP Batch Streamer",
        "labels": "milestone-2,subsystem:android",
        "body": """## Objective & Scope
Query Android `MediaStore` camera roll, generate in-memory 256x256 WebP thumbnails, and stream to desktop cache.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 3
- Reference: `docs/ANDROID_IMPLEMENTATION.md` Section 5
- Query: `MediaStore.Images.Media.EXTERNAL_CONTENT_URI`

## Technical Requirements & Constraints
- Cursor-based query ordered by `DATE_ADDED DESC`.
- Use `ContentResolver.loadThumbnail` with 256x256 size.
- Compress to WebP bitmap bytes in memory.
- Desktop daemon caches thumbnails to `~/.cache/omnilink/thumbnails/<photo_id>.webp`.

## Acceptance Criteria
- [ ] Initial connection indexes up to 500 recent photos in under 3 seconds.
- [ ] Thumbnails correctly written to desktop local cache directory.

## Dependencies & Blockers
- Prerequisite: Issue 2E, Issue 6A.

## Out of Scope
- Full-resolution photo transfers (on demand when clicked) or GUI grid.

## Verification & Testing
```bash
./gradlew :app:testDebugUnitTest
```"""
    },
    {
        "title": "Issue 6C: [GUI] Photo Gallery Grid & Desktop Drag-and-Drop",
        "labels": "milestone-2,subsystem:gui",
        "body": """## Objective & Scope
Build virtualized masonry photo gallery in Tauri GUI and enable native desktop drag-out file exports.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 3
- Reference: `docs/ARCHITECTURE.md` Section 3
- Panel: Photos Tab in `omnilink-gui`.

## Technical Requirements & Constraints
- Virtualized scrolling grid displaying local cached WebP thumbnails.
- Clicking thumbnail opens high-res modal viewer (fetching full image on demand).
- Dragging a photo out of the window initiates native OS drag-and-drop file copy to Linux apps (GIMP, Discord, nautilus).

## Acceptance Criteria
- [ ] Opening Photos tab displays instant grid with zero UI jank.
- [ ] Dragging photo to file manager saves full resolution image file.

## Dependencies & Blockers
- Prerequisite: Issue 5B, Issue 6B.

## Out of Scope
- In-place photo editing or deletion.

## Verification & Testing
- Drag thumbnail to terminal or file manager and verify full-res file drop."""
    },
    {
        "title": "Issue 8B: [Daemon] Hardware Video Decoder (VA-API/NVDEC) & Wayland Mirror Window",
        "labels": "milestone-3,subsystem:wayland",
        "body": """## Objective & Scope
Decode incoming H.264 screen stream with hardware acceleration and render directly into floating Wayland window.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 4
- Window Class: `omnilink-mirror`
- Libraries: GStreamer Rust bindings (`gstreamer-rs`) or VA-API/NVDEC bindings.

## Technical Requirements & Constraints
- Receive H.264 NAL units from Stream 4.
- Hardware decoding pipeline with zero-copy Wayland surface presentation.
- End-to-end latency target: Sub-40ms at 1080p 60fps.
- Respect Hyprland floating rules (`windowrulev2 = float, class:^(omnilink-mirror)$`).

## Acceptance Criteria
- [ ] Window opens and displays live video feed at 60fps.
- [ ] CPU utilization stays under 10% during active 1080p stream due to hardware acceleration.

## Dependencies & Blockers
- Prerequisite: Issue 8A.

## Out of Scope
- Capturing user input and injecting into phone (handled in Issue 8C).

## Verification & Testing
```bash
cargo test -p omnilink-daemon video_decode
```"""
    },
    {
        "title": "Issue 8C: [Integration] Remote Input Capture & Injection Protocol (Stream 5)",
        "labels": "milestone-3,subsystem:wayland,subsystem:android",
        "body": """## Objective & Scope
Capture mouse clicks, touch swipes, and keyboard typing from mirror window and inject into Android via scrcpy protocol.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 4
- Stream: Dedicated Stream 5.

## Technical Requirements & Constraints
- Translate Wayland pointer events to normalized coordinates `(x / width, y / height)`.
- Support left click, right click (mapped to Back), scroll wheel, and touch drag gestures.
- Inject keystrokes into scrcpy control socket on Android.

## Acceptance Criteria
- [ ] Clicking on desktop mirror window interacts with phone UI with no noticeable lag.
- [ ] Typing in focused mirror window inputs text directly into active Android text field.

## Dependencies & Blockers
- Prerequisite: Issue 8B.

## Out of Scope
- Virtual display multi-window routing (handled in Issue 9B).

## Verification & Testing
- Interact with phone home screen and keyboard inside mirror window."""
    },
    {
        "title": "Issue 9B: [Daemon/GUI] Virtual Display App Session Manager & Desktop Launcher",
        "labels": "milestone-3,subsystem:wayland,subsystem:gui",
        "body": """## Objective & Scope
Build App Drawer in Tauri GUI and manage dedicated floating streaming windows for individual Android apps.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 4
- Reference: `docs/ARCHITECTURE.md` Section 3 & 4
- IPC Method: `mirror.launch { package_name: String }`

## Technical Requirements & Constraints
- Query installed applications list from phone via Stream 0.
- App Drawer tab in GUI showing app icons and titles.
- Clicking an app requests daemon to launch virtual display and stream only that application to a dedicated window.

## Acceptance Criteria
- [ ] Clicking app in drawer opens single-app floating window.
- [ ] Phone user can simultaneously use another app on the physical phone screen without interference.

## Dependencies & Blockers
- Prerequisite: Issue 8C, Issue 9A.

## Out of Scope
- Desktop audio routing for the streamed app (handled via PipeWire in Milestone 4).

## Verification & Testing
- Launch Spotify or messaging app from App Drawer and verify independent execution."""
    },
    {
        "title": "Issue 10B: [Daemon] Telephony State Machine & IPC Call Control",
        "labels": "milestone-4,subsystem:wayland",
        "body": """## Objective & Scope
Track incoming call states from Android/BlueZ and expose RPC endpoints (`call.answer`, `call.hangup`).

## Architecture & References
- Reference: `docs/IPC_SPECIFICATION.md` Section 2 & 3
- Events: `telephony.incoming_call { caller_name, phone_number }`

## Technical Requirements & Constraints
- Monitor BlueZ telephony interface / Android telephony listener.
- Broadcast incoming call event to connected IPC clients.
- Provide RPC methods: `call.answer`, `call.hangup`, `call.send_sms { phone_number, message }`.

## Acceptance Criteria
- [ ] Incoming call triggers `telephony.incoming_call` event over `/run/user/<UID>/omnilink.sock`.
- [ ] Calling `call.hangup` disconnects ringing call on phone.

## Dependencies & Blockers
- Prerequisite: Issue 5A, Issue 10A.

## Out of Scope
- Audio node routing (Issue 10A) or GUI dialer widget (Issue 10C).

## Verification & Testing
```bash
cargo test -p omnilink-daemon telephony_state
```"""
    },
    {
        "title": "Issue 10C: [GUI] Desktop Incoming Call Banner & In-App Dialer Pad",
        "labels": "milestone-4,subsystem:gui",
        "body": """## Objective & Scope
Build incoming call floating alert banner and in-app keypad dialer in Tauri desktop GUI.

## Architecture & References
- Reference: `docs/SUBSYSTEMS.md` Section 5
- Reference: `docs/ARCHITECTURE.md` Section 3

## Technical Requirements & Constraints
- Banner: Displays Caller ID and buttons: "Answer", "Decline", "Quick SMS".
- Dialer: Numeric keypad (0-9, *, #) with contact search and call button.
- Send DTMF tones during active call.

## Acceptance Criteria
- [ ] Incoming call pops up banner on desktop screen.
- [ ] Clicking Answer answers call; audio routes through headset.
- [ ] Keypad initiates outgoing call through phone cellular connection.

## Dependencies & Blockers
- Prerequisite: Issue 5B, Issue 10B.

## Out of Scope
- Direct SIP or VoIP bypass (calls always bridge through paired phone).

## Verification & Testing
- Trigger mock incoming call event and verify banner and action handlers."""
    }
]

def main():
    print(f"=== Starting OmniLink GitHub Issue Synchronization ===")
    
    # Update existing 10 issues
    print(f"\n--- Updating 10 Existing Issues ---")
    for item in ISSUES_TO_UPDATE:
        num = item["num"]
        title = item["title"]
        labels = item["labels"]
        body = item["body"]
        cmd = [
            GH_BIN, "issue", "edit", str(num),
            "--repo", REPO,
            "--title", title,
            "--body", body
        ]
        ok, out = run_cmd(cmd)
        if not ok:
            print(f"Failed to edit issue #{num}")
        time.sleep(1)

    # Create 21 new split issues
    print(f"\n--- Creating 21 New Sub-Issues ---")
    created_issues = []
    for item in NEW_ISSUES_TO_CREATE:
        title = item["title"]
        labels = item["labels"]
        body = item["body"]
        cmd = [
            GH_BIN, "issue", "create",
            "--repo", REPO,
            "--title", title,
            "--label", labels,
            "--body", body
        ]
        ok, out = run_cmd(cmd)
        if ok:
            created_issues.append((title, out))
        else:
            print(f"Failed to create issue: {title}")
        time.sleep(1)

    print(f"\n=== Synchronization Complete ===")
    print(f"Updated 10 issues.")
    print(f"Created {len(created_issues)} new issues.")

if __name__ == "__main__":
    main()
