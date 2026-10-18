#!/usr/bin/env bash
set -euo pipefail

REPO="manideepanasuri/omnilink"

echo "Creating GitHub issues for $REPO..."

# Label setup
gh label create "milestone-1" --color "0E8A16" --description "Milestone 1: Core Foundation & Daily Essentials" --repo "$REPO" --force || true
gh label create "milestone-2" --color "1D76DB" --description "Milestone 2: Desktop GUI & Media Transfer" --repo "$REPO" --force || true
gh label create "milestone-3" --color "5319E7" --description "Milestone 3: Screen Mirroring & App Streaming" --repo "$REPO" --force || true
gh label create "milestone-4" --color "D93F0B" --description "Milestone 4: Telephony & Call Audio" --repo "$REPO" --force || true
gh label create "subsystem:protocol" --color "C5DEF5" --description "Protocol & Transport" --repo "$REPO" --force || true
gh label create "subsystem:wayland" --color "BFD4F2" --description "Wayland & Hyprland Integration" --repo "$REPO" --force || true
gh label create "subsystem:android" --color "006B75" --description "Android Companion App" --repo "$REPO" --force || true
gh label create "subsystem:gui" --color "F9D0C4" --description "Tauri Desktop GUI" --repo "$REPO" --force || true

# Issue 1
gh issue create --repo "$REPO" \
  --title "Issue 1: Project Workspace Scaffolding & CI Pipeline" \
  --label "milestone-1,subsystem:protocol" \
  --body "## Summary
Scaffold the multi-crate Rust workspace and Android companion project structure.

## Tasks
- [ ] Initialize Cargo workspace root \`Cargo.toml\`.
- [ ] Create crates: \`crates/omnilink-protocol\`, \`crates/omnilink-daemon\`, \`crates/omnilink-cli\`, \`crates/omnilink-gui\`.
- [ ] Initialize Android Kotlin project in \`android/\` with Jetpack Compose and Gradle KTS.
- [ ] Add GitHub Actions CI workflow for \`cargo fmt\`, \`cargo clippy\`, \`cargo test\`, and Android build validation.

## Acceptance Criteria
- \`cargo check --workspace\` succeeds without warnings.
- Android project builds clean debug APK with \`./gradlew assembleDebug\`."

# Issue 2
gh issue create --repo "$REPO" \
  --title "Issue 2: Core Protocol, Ed25519 Pairing Handshake & QUIC Transport" \
  --label "milestone-1,subsystem:protocol" \
  --body "## Summary
Implement mutual TLS / Noise-authenticated QUIC transport and QR code pairing.

## Tasks
- [ ] Generate and securely store Ed25519 keypairs (\`~/.local/share/omnilink/identity.key\`).
- [ ] Implement terminal / GUI QR code generator with local IP addresses, port, and one-time secret.
- [ ] Implement QR code camera scanner in Android companion.
- [ ] Build multiplexed QUIC endpoint listener (\`quinn\`) on port 42424 with periodic heartbeats.
- [ ] Implement BLE advertisement beacon with service UUID \`6f6d-6e69-6c69-6e6b-0001\`.

## Acceptance Criteria
- Scanning Linux QR code on Android establishes mutual authenticated QUIC session within 500ms.
- Session automatically reconnects when phone returns to Wi-Fi."

# Issue 3
gh issue create --repo "$REPO" \
  --title "Issue 3: Bidirectional Wayland Clipboard Synchronization" \
  --label "milestone-1,subsystem:wayland,subsystem:android" \
  --body "## Summary
Seamless, zero-latency clipboard synchronization between Android and Linux Wayland.

## Tasks
- [ ] Implement Wayland selection listener using \`ext-data-control-v1\` / \`wlr-data-control-unstable-v1\`.
- [ ] Implement BLAKE3 circular buffer hash filter to prevent infinite echo loops.
- [ ] Implement Android \`ClipboardManager\` listener and injector in \`OmnilinkSyncService\`.
- [ ] Support MIME types: \`text/plain;charset=utf-8\` and \`image/png\`.
- [ ] Respect \`ClipDescription.EXTRA_IS_SENSITIVE\` and password manager markers.

## Acceptance Criteria
- Copying text/image on phone is immediately pastable on Linux (\`wl-paste\`).
- Copying on Linux immediately updates Android clipboard without re-triggering Linux."

# Issue 4
gh issue create --repo "$REPO" \
  --title "Issue 4: Two-Way Desktop Notifications & Inline RemoteInput Replies" \
  --label "milestone-1,subsystem:android,subsystem:wayland" \
  --body "## Summary
Forward Android notifications to Linux desktop with inline replies and two-way dismissal sync.

## Tasks
- [ ] Implement \`NotificationListenerService\` extracting title, text, icon, and \`RemoteInput\` actions.
- [ ] Connect \`omnilink-daemon\` to D-Bus \`org.freedesktop.Notifications\` using \`zbus\`.
- [ ] Handle notification action callbacks on Linux to trigger Android \`RemoteInput.addResultsToIntent\`.
- [ ] Sync notification dismissals in both directions.
- [ ] Implement local app blacklist in \`~/.config/omnilink/config.toml\`.

## Acceptance Criteria
- Messaging notifications (WhatsApp, Telegram, SMS) pop up natively on Omarchy/Hyprland.
- Typing a reply in the desktop notification bubble sends the message via Android.
- Dismissing on desktop clears notification from phone status bar."

# Issue 5
gh issue create --repo "$REPO" \
  --title "Issue 5: Desktop GUI Dashboard (Tauri v2 + Svelte + Tailwind)" \
  --label "milestone-2,subsystem:gui" \
  --body "## Summary
Build modern, lightweight desktop application providing Windows Phone Link visual polish.

## Tasks
- [ ] Initialize Tauri v2 application in \`crates/omnilink-gui\`.
- [ ] Implement UNIX domain socket client connecting to \`/run/user/<UID>/omnilink.sock\`.
- [ ] Build UI tabs: Overview (battery, Wi-Fi, signal, toggles), Photos, Messages, Apps.
- [ ] Apply dark-mode styling consistent with Omarchy and Hyprland aesthetics.

## Acceptance Criteria
- Launching \`omnilink-gui\` opens responsive dashboard idling at under 45MB RAM.
- Real-time battery and network status update without manual refresh."

# Issue 6
gh issue create --repo "$REPO" \
  --title "Issue 6: Thumbnail-Cached Photo Gallery & High-Speed File Transfer" \
  --label "milestone-2,subsystem:gui,subsystem:android" \
  --body "## Summary
Instant photo browsing with thumbnail caching and drag-and-drop file transfer.

## Tasks
- [ ] Query Android \`MediaStore\` and generate 256x256 WebP thumbnails in memory.
- [ ] Cache thumbnails locally on Linux at \`~/.cache/omnilink/thumbnails/\`.
- [ ] Support dragging photos directly from Tauri gallery out to Linux desktop apps.
- [ ] Implement chunked file transfer over QUIC Stream 3 for \`omnilink send <file>\`.
- [ ] Verify BLAKE3 checksum post-transfer.

## Acceptance Criteria
- Opening Photos tab shows instant grid of recent camera photos from local cache.
- Dragging photo to GIMP or Discord fetches and drops full-res image instantly.
- Running \`omnilink send file.pdf\` deposits file in \`/sdcard/Download/\`."

# Issue 7
gh issue create --repo "$REPO" \
  --title "Issue 7: Waybar & Hyprland Integration Modules" \
  --label "milestone-2,subsystem:wayland" \
  --body "## Summary
Native status bar modules and window rules for Hyprland and Omarchy.

## Tasks
- [ ] Implement \`omnilink status --watch-waybar\` outputting streaming JSON.
- [ ] Add CLI commands: \`omnilink toggle-hotspot\`, \`omnilink ring\`, \`omnilink mirror\`.
- [ ] Configure recommended Hyprland window rules (\`windowrulev2\`) for floating mirrors and dialogs.

## Acceptance Criteria
- Waybar displays phone battery percentage and connection status.
- Left-clicking Waybar module opens GUI; right-clicking launches screen mirror."

# Issue 8
gh issue create --repo "$REPO" \
  --title "Issue 8: Embedded scrcpy Screen Mirroring & Hardware Decoding" \
  --label "milestone-3,subsystem:wayland,subsystem:android" \
  --body "## Summary
Low-latency (25-40ms) screen mirroring with hardware video decoding and input injection.

## Tasks
- [ ] Package and execute \`scrcpy-server.jar\` via Shizuku / Wireless ADB.
- [ ] Stream H.264 video at 60fps over dedicated QUIC / TCP stream.
- [ ] Implement hardware decoding on Linux via VA-API / NVDEC.
- [ ] Map mouse clicks, touch swipes, and keyboard typing to scrcpy input protocol.
- [ ] Render video directly onto floating Wayland window (\`omnilink-mirror\`).

## Acceptance Criteria
- Screen mirror launches in under 1 second with sub-40ms latency.
- Full mouse and keyboard control functional inside mirror window."

# Issue 9
gh issue create --repo "$REPO" \
  --title "Issue 9: Single-App Streaming via Virtual Display" \
  --label "milestone-3,subsystem:android,subsystem:wayland" \
  --body "## Summary
Stream individual Android apps into isolated floating desktop windows.

## Tasks
- [ ] Implement Android secondary virtual display manager using Shizuku privileged APIs.
- [ ] Launch target application onto virtual display (\`am start-activity --display <id> <package>\`).
- [ ] Route isolated virtual display video feed to dedicated Hyprland window.
- [ ] Maintain independent input focus without locking phone's primary screen.

## Acceptance Criteria
- Clicking an app in Omnilink app drawer opens only that app in a dedicated desktop window.
- Using the streamed app on PC does not interrupt phone use."

# Issue 10
gh issue create --repo "$REPO" \
  --title "Issue 10: Bluetooth HFP & PipeWire Call Audio Routing" \
  --label "milestone-4,subsystem:wayland" \
  --body "## Summary
Answer and make phone calls using Linux PC microphone and speakers via PipeWire.

## Tasks
- [ ] Register Bluetooth Hands-Free Profile (HFP/HSP) via BlueZ (\`org.bluez\`).
- [ ] Configure PipeWire audio links to map Bluetooth SCO audio to default audio sink/source.
- [ ] Build desktop incoming call banner with Answer, Decline, and Quick SMS.
- [ ] Implement dialer pad in Tauri GUI.

## Acceptance Criteria
- Incoming call triggers desktop alert with Caller ID.
- Clicking Answer routes two-way clear audio through PC mic and headset."

echo "All 10 GitHub issues created successfully!"
