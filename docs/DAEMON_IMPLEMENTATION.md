# Omnilink Daemon: Technical Implementation Guide

This document provides concrete code patterns, crate dependencies, module structures, and event loop logic for `omnilink-daemon`.

---

## 1. Crate Architecture & Dependencies

The daemon is written in asynchronous Rust using `tokio`.

### `Cargo.toml`
```toml
[package]
name = "omnilink-daemon"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async Runtime
tokio = { version = "1.40", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }
futures = "0.3"

# Networking & Security
quinn = "0.11"
rcgen = "0.13"
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
blake3 = "1.5"
ed25519-dalek = { version = "2.1", features = ["rand_core"] }

# Linux Desktop Integration
zbus = { version = "5.0", features = ["tokio"] }
wayland-client = "0.31"
wayland-protocols = { version = "0.32", features = ["client"] }
wayland-protocols-wlr = { version = "0.3", features = ["client"] }
bluer = { version = "0.17", features = ["bluetoothd"] }
pipewire = { version = "0.8", optional = true }

# Serialization & CLI
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
rmp-serde = "1.3"
directories = "5.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "1.0"
anyhow = "1.0"
```

---

## 2. Daemon Lifecycle & State Machine

```
[Systemd User Service Start]
            │
            ▼
   Initialize Logging & Config (~/.config/omnilink/config.toml)
            │
            ▼
   Load Identity Keypair (Ed25519 from ~/.local/share/omnilink/identity.key)
            │
            ├──▶ Spawn Unix IPC Server (/run/user/$UID/omnilink.sock)
            ├──▶ Spawn Wayland Clipboard Listener (ext-data-control)
            ├──▶ Spawn D-Bus Notification Listener (zbus)
            ├──▶ Spawn BLE Scanner (bluer)
            └──▶ Bind QUIC Endpoint (0.0.0.0:42424)
                        │
                        ▼
            Connection Established (Authenticated Phone)
                        │
                        ├── Stream 0 Task: Control & Telemetry Heartbeats
                        ├── Stream 1 Task: Clipboard Sync Pump
                        ├── Stream 2 Task: Notification & RemoteInput Pump
                        └── Stream 3 Task: File Transfer Dispatcher
```

### Shared Daemon State (`src/state.rs`)
```rust
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

pub struct DaemonState {
    pub config: Config,
    pub identity: Keypair,
    pub paired_device: RwLock<Option<PairedDevice>>,
    pub session: RwLock<Option<ActiveSession>>,
    pub clipboard_tx: mpsc::Sender<ClipboardEvent>,
    pub ipc_broadcast_tx: broadcast::Sender<IpcEvent>,
}

pub struct ActiveSession {
    pub device_id: String,
    pub quic_connection: quinn::Connection,
    pub battery_level: u8,
    pub is_charging: bool,
    pub wifi_ssid: Option<String>,
}
```

---

## 3. Wayland Clipboard Implementation (`src/clipboard.rs`)

Omnilink uses Wayland's `ext-data-control-v1` / `wlr-data-control-unstable-v1` protocol to listen to clipboard selections and inject foreign selections without stealing focus.

### Loop Detection & Hash Ring
```rust
use std::collections::VecDeque;
use blake3::Hash;

pub struct ClipboardFilter {
    recent_hashes: VecDeque<Hash>,
    max_history: usize,
}

impl ClipboardFilter {
    pub fn new() -> Self {
        Self {
            recent_hashes: VecDeque::with_capacity(16),
            max_history: 16,
        }
    }

    pub fn should_sync(&mut self, data: &[u8]) -> bool {
        let hash = blake3::hash(data);
        if self.recent_hashes.contains(&hash) {
            return false;
        }
        if self.recent_hashes.len() >= self.max_history {
            self.recent_hashes.pop_front();
        }
        self.recent_hashes.push_back(hash);
        true
    }
}
```

---

## 4. D-Bus Desktop Notifications (`src/notifications.rs`)

Omnilink uses `zbus` to integrate with `org.freedesktop.Notifications`.

### Forwarding to Desktop & Capturing Reply
```rust
use zbus::{Connection, proxy};

#[proxy(
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications",
    interface = "org.freedesktop.Notifications"
)]
trait FreedesktopNotifications {
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: &std::collections::HashMap<&str, zbus::zvariant::Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
}
```

### Action Invocation Mapping
- Action ID `"inline-reply"` is passed if `can_reply == true`.
- Quickshell / modern notification servers emit `ActionInvoked(id, "inline-reply")` or `NotificationReplied(id, reply_text)`.
- The daemon catches this signal, maps `id` to the mobile `notification_key`, and serializes a `NotificationReplyPacket` back into QUIC Stream 2.

---

## 5. UNIX Socket IPC Server (`src/ipc.rs`)

Local applications (`omnilink-cli`, `omnilink-gui`, Waybar scripts) communicate with the daemon via `/run/user/<UID>/omnilink.sock`.

### Framing & Serialization
- Messages are delimited by newline (`\n`) using `serde_json`.
- Supports request-response (RPC) and unidirectional publish-subscribe events.
