# Omnilink

Omnilink is a high-performance Linux phone integration platform tailored for Wayland environments (specifically Omarchy / Hyprland) paired with Samsung Galaxy (and modern Android) devices.

It bridges the gap left by KDE Connect by providing the seamless, zero-latency experience of Windows Phone Link: instant bidirectional clipboard, two-way notifications with inline replies, thumbnail-cached photo browsing, drag-and-drop file transfers, low-latency app/screen streaming, and Bluetooth call handling.

## Directory Layout

```
omnilink/
├── Cargo.toml                  # Rust workspace definition
├── README.md                   # Project overview & quickstart
├── docs/                       # Technical specifications for agents & developers
│   ├── ARCHITECTURE.md         # System topology, daemons, and components
│   ├── PROTOCOL.md             # QUIC transport, framing, security, and packets
│   ├── SUBSYSTEMS.md           # Clipboard, notification, scrcpy, and audio specs
│   ├── DAEMON_IMPLEMENTATION.md# Rust daemon loop, crate deps, and zbus/wayland code
│   ├── ANDROID_IMPLEMENTATION.md# Kotlin service, Shizuku, RemoteInput, and MediaStore
│   ├── IPC_SPECIFICATION.md    # UNIX domain socket JSON-RPC and Waybar schemas
│   └── ROADMAP.md              # Milestones and acceptance criteria
├── crates/
│   ├── omnilink-protocol/      # Shared packet serialization & crypto types
│   ├── omnilink-daemon/        # Background Linux service (D-Bus, Wayland, PipeWire)
│   ├── omnilink-cli/           # CLI tool & Waybar / Quickshell status provider
│   └── omnilink-gui/           # Tauri v2 desktop application (Svelte/Tailwind)
└── android/                    # Native Android companion application (Kotlin/Compose)
```

## Core Tech Stack

- **Linux Host**: Rust (Tokio, zbus, bluer, wayland-client, pipewire-rs, quinn).
- **Desktop UI**: Tauri v2 with Svelte & Tailwind CSS.
- **Android Companion**: Kotlin, Jetpack Compose, NotificationListenerService, Shizuku / Wireless ADB integration.
- **Transport**: Multiplexed QUIC over UDP with Noise/TLS encryption.
- **Screen Engine**: Embedded `scrcpy` protocol with VA-API / NVDEC hardware decoding.
