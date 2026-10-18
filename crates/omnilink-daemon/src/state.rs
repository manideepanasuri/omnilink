//! Daemon and active connection session state.

use ed25519_dalek::{SigningKey, VerifyingKey};
use omnilink_protocol::packet::{ClipboardPacket, ControlPacket};
use omnilink_protocol::types::DeviceStatus;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::clipboard::{ClipboardEvent, ClipboardFilter};

/// Information about a paired companion device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDevice {
    /// Human-readable device name (e.g. "Pixel 8 Pro").
    pub name: String,
    /// Paired Ed25519 public verifying key.
    pub public_key: VerifyingKey,
    /// Unix timestamp in seconds when pairing was established.
    pub paired_at: u64,
}

/// Active QUIC session and live telemetry for a connected companion device.
#[derive(Debug, Clone)]
pub struct ActiveSession {
    /// Identifier of the connected device (e.g. Base64 public key or device name).
    pub device_id: String,
    /// Live QUIC connection handle.
    pub connection: quinn::Connection,
    /// Stable connection identifier for race condition guards.
    pub connection_id: usize,
    /// Outbound command dispatch channel for Stream 0.
    pub command_tx: tokio::sync::mpsc::Sender<ControlPacket>,
    /// Outbound clipboard dispatch channel for Stream 1.
    pub clipboard_tx: tokio::sync::mpsc::Sender<ClipboardEvent>,
    /// Device battery charge percentage (0..100).
    pub battery_pct: u8,
    /// Whether the device is connected to power/charging.
    pub is_charging: bool,
    /// Connected Wi-Fi network SSID, if available.
    pub wifi_ssid: Option<String>,
    /// Cellular network signal strength in bars (0..4).
    pub cellular_signal: u8,
    /// Whether the device's mobile hotspot is currently active.
    pub hotspot_active: bool,
    /// Instant when the last valid control packet/heartbeat was received.
    pub last_heartbeat: Instant,
}

impl ActiveSession {
    /// Creates a new `ActiveSession` with default zeroed telemetry.
    pub fn new(
        device_id: impl Into<String>,
        connection: quinn::Connection,
        command_tx: tokio::sync::mpsc::Sender<ControlPacket>,
        clipboard_tx: tokio::sync::mpsc::Sender<ClipboardEvent>,
    ) -> Self {
        let connection_id = connection.stable_id();
        Self {
            device_id: device_id.into(),
            connection,
            connection_id,
            command_tx,
            clipboard_tx,
            battery_pct: 0,
            is_charging: false,
            wifi_ssid: None,
            cellular_signal: 0,
            hotspot_active: false,
            last_heartbeat: Instant::now(),
        }
    }

    /// Returns the live QUIC connection handle.
    pub fn quic_connection(&self) -> &quinn::Connection {
        &self.connection
    }

    /// Returns the connection's stable ID.
    pub fn stable_id(&self) -> usize {
        self.connection_id
    }

    /// Sends an outbound command packet to the companion over Stream 0.
    pub async fn send_command(
        &self,
        packet: ControlPacket,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ControlPacket>> {
        self.command_tx.send(packet).await
    }

    /// Returns a reference to the outbound clipboard channel sender for Stream 1.
    pub fn clipboard_sender(&self) -> &tokio::sync::mpsc::Sender<ClipboardEvent> {
        &self.clipboard_tx
    }

    /// Sends an outbound clipboard event to the companion over Stream 1.
    pub async fn send_clipboard(
        &self,
        event: ClipboardEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ClipboardEvent>> {
        self.clipboard_tx.send(event).await
    }

    /// Updates session telemetry fields from a received `DeviceStatus`.
    pub fn update_status(&mut self, status: DeviceStatus) {
        self.battery_pct = status.battery_pct;
        self.is_charging = status.is_charging;
        self.wifi_ssid = status.wifi_ssid;
        self.cellular_signal = status.cellular_signal;
        self.hotspot_active = status.hotspot_active;
        self.last_heartbeat = Instant::now();
    }

    /// Returns the current telemetry snapshot as a `DeviceStatus`.
    pub fn to_device_status(&self) -> DeviceStatus {
        DeviceStatus {
            battery_pct: self.battery_pct,
            is_charging: self.is_charging,
            wifi_ssid: self.wifi_ssid.clone(),
            cellular_signal: self.cellular_signal,
            hotspot_active: self.hotspot_active,
        }
    }

    /// Refreshes the last heartbeat timestamp.
    pub fn touch_heartbeat(&mut self) {
        self.last_heartbeat = Instant::now();
    }
}

/// Global shared state managed by the Omnilink daemon.
pub struct DaemonState {
    /// Local daemon Ed25519 signing keypair.
    pub identity: SigningKey,
    /// Paired companion device credentials, if paired.
    pub paired_device: RwLock<Option<PairedDevice>>,
    /// Shared verifying key container linked directly with the QUIC TLS verifier.
    pub paired_verifier_key: Arc<std::sync::RwLock<Option<VerifyingKey>>>,
    /// Active authenticated connection session, if connected.
    pub session: RwLock<Option<ActiveSession>>,
    /// BLAKE3 circular buffer filter for loop and echo detection.
    pub clipboard_filter: Arc<std::sync::Mutex<ClipboardFilter>>,
    /// Last received inbound clipboard packet from remote companion.
    pub last_received_clipboard: RwLock<Option<ClipboardPacket>>,
    /// Inbound clipboard notification broadcast channel.
    pub inbound_clipboard_tx: tokio::sync::broadcast::Sender<ClipboardPacket>,
}

/// Type alias for `DaemonState` to support `SessionState` naming convention.
pub type SessionState = DaemonState;

impl DaemonState {
    /// Creates a new `DaemonState` with the provided identity and optional paired device.
    pub fn new(identity: SigningKey, paired_device: Option<PairedDevice>) -> Self {
        let initial_key = paired_device.as_ref().map(|d| d.public_key);
        let (inbound_clipboard_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            identity,
            paired_device: RwLock::new(paired_device),
            paired_verifier_key: Arc::new(std::sync::RwLock::new(initial_key)),
            session: RwLock::new(None),
            clipboard_filter: Arc::new(std::sync::Mutex::new(ClipboardFilter::new())),
            last_received_clipboard: RwLock::new(None),
            inbound_clipboard_tx,
        }
    }

    /// Creates a new `DaemonState` sharing a pre-existing `ClipboardFilter`.
    pub fn with_filter(
        identity: SigningKey,
        paired_device: Option<PairedDevice>,
        clipboard_filter: Arc<std::sync::Mutex<ClipboardFilter>>,
    ) -> Self {
        let initial_key = paired_device.as_ref().map(|d| d.public_key);
        let (inbound_clipboard_tx, _) = tokio::sync::broadcast::channel(32);
        Self {
            identity,
            paired_device: RwLock::new(paired_device),
            paired_verifier_key: Arc::new(std::sync::RwLock::new(initial_key)),
            session: RwLock::new(None),
            clipboard_filter,
            last_received_clipboard: RwLock::new(None),
            inbound_clipboard_tx,
        }
    }

    /// Loads the daemon identity from default keystore (or generates a new one)
    /// and initializes daemon state.
    pub fn with_defaults() -> Result<Self, crate::identity::IdentityError> {
        let identity = crate::identity::load_or_generate_identity(None)?;
        Ok(Self::new(identity, None))
    }

    /// Returns true if an active companion device connection is currently established.
    pub async fn is_connected(&self) -> bool {
        self.session.read().await.is_some()
    }

    /// Returns the paired device's public key if paired.
    pub async fn paired_public_key(&self) -> Option<VerifyingKey> {
        self.paired_device
            .read()
            .await
            .as_ref()
            .map(|d| d.public_key)
    }

    /// Sets or removes the paired device, dynamically updating the shared TLS verifier key.
    pub async fn set_paired_device(&self, device: Option<PairedDevice>) {
        let key = device.as_ref().map(|d| d.public_key);
        if let Ok(mut lock) = self.paired_verifier_key.write() {
            *lock = key;
        }
        *self.paired_device.write().await = device;
    }

    /// Returns a telemetry snapshot from the active session, if connected.
    pub async fn get_device_status(&self) -> Option<DeviceStatus> {
        self.session.read().await.as_ref().map(|s| s.to_device_status())
    }

    /// Dispatches an outbound command packet to the companion over Stream 0, if connected.
    pub async fn send_command(
        &self,
        packet: ControlPacket,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ControlPacket>> {
        let session_lock = self.session.read().await;
        if let Some(ref session) = *session_lock {
            session.send_command(packet).await
        } else {
            Err(tokio::sync::mpsc::error::SendError(packet))
        }
    }

    /// Returns a reference to the shared clipboard filter.
    pub fn clipboard_filter(&self) -> &Arc<std::sync::Mutex<ClipboardFilter>> {
        &self.clipboard_filter
    }

    /// Dispatches an outbound clipboard event to the companion over Stream 1, if connected.
    pub async fn send_clipboard(
        &self,
        event: ClipboardEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ClipboardEvent>> {
        let session_lock = self.session.read().await;
        if let Some(ref session) = *session_lock {
            session.send_clipboard(event).await
        } else {
            Err(tokio::sync::mpsc::error::SendError(event))
        }
    }

    /// Dispatches an outbound clipboard event to the companion over Stream 1 (alias for `send_clipboard`).
    pub async fn send_clipboard_event(
        &self,
        event: ClipboardEvent,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<ClipboardEvent>> {
        self.send_clipboard(event).await
    }

    /// Returns a clone of the outbound clipboard sender for the active session, if connected.
    pub async fn clipboard_tx(&self) -> Option<tokio::sync::mpsc::Sender<ClipboardEvent>> {
        self.session.read().await.as_ref().map(|s| s.clipboard_tx.clone())
    }

    /// Records an inbound clipboard packet from remote companion into local filter and state.
    pub async fn notify_inbound_clipboard(&self, packet: ClipboardPacket) {
        {
            let mut filter = self.clipboard_filter.lock().unwrap();
            filter.record(&packet.payload);
        }
        let mut last = self.last_received_clipboard.write().await;
        *last = Some(packet.clone());
        let _ = self.inbound_clipboard_tx.send(packet);
    }

    /// Returns the last received inbound clipboard packet, if any.
    pub async fn last_received_clipboard(&self) -> Option<ClipboardPacket> {
        self.last_received_clipboard.read().await.clone()
    }

    /// Subscribes to inbound clipboard packets received from the remote companion.
    pub fn subscribe_inbound_clipboard(&self) -> tokio::sync::broadcast::Receiver<ClipboardPacket> {
        self.inbound_clipboard_tx.subscribe()
    }

    /// Drops the active session only if the provided stable_id matches the current active connection.
    pub async fn clear_session(&self, stable_id: usize) {
        let mut session_lock = self.session.write().await;
        if let Some(ref current_session) = *session_lock {
            if current_session.connection.stable_id() == stable_id {
                *session_lock = None;
            }
        }
    }
}
