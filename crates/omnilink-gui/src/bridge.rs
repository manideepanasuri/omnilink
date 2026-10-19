//! UNIX domain socket IPC client bridge for omnilink-gui.
//!
//! Connects to `/run/user/<UID>/omnilink.sock` (or fallback / override path)
//! and multiplexes JSON-RPC 2.0 requests while streaming real-time broadcast
//! events to the Tauri frontend.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use omnilink_daemon::ipc::{
    default_socket_path, JsonRpcRequest, JsonRpcResponse, StatusGetResult,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tracing::{debug, info, warn};

/// Errors originating from the IPC bridge operations.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Daemon IPC socket is not connected.
    #[error("Daemon IPC socket not connected")]
    NotConnected,

    /// Underlying I/O error on UNIX domain socket.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// JSON-RPC response returned an error object.
    #[error("RPC error (code {code}): {message}")]
    Rpc { code: i32, message: String },

    /// RPC request timed out awaiting response.
    #[error("Request timed out")]
    Timeout,

    /// Internal channel was closed unexpectedly.
    #[error("Internal channel closed")]
    ChannelClosed,
}

/// Abstract event emitter for forwarding broadcast events to Tauri frontend or test mocks.
pub trait EventEmitter: Send + Sync + 'static {
    /// Emits a named event with a JSON payload.
    fn emit(&self, event: &str, payload: Value) -> Result<(), String>;
}

/// Tauri event emitter implementation wrapping AppHandle.
pub struct TauriEventEmitter<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriEventEmitter<R> {
    /// Constructs a new Tauri event emitter with the provided AppHandle.
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> EventEmitter for TauriEventEmitter<R> {
    fn emit(&self, event: &str, payload: Value) -> Result<(), String> {
        use tauri::Emitter;
        self.app.emit(event, payload).map_err(|e| e.to_string())
    }
}

/// Mock event emitter for unit and integration testing without a Tauri runtime.
#[derive(Default, Clone)]
pub struct MockEventEmitter {
    /// Thread-safe collection of recorded (event, payload) pairs.
    pub events: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
}

impl MockEventEmitter {
    /// Creates an empty mock event emitter.
    pub fn new() -> Self {
        Self {
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    /// Returns a copy of all emitted events recorded so far.
    pub fn recorded_events(&self) -> Vec<(String, Value)> {
        self.events.lock().unwrap().clone()
    }
}

impl EventEmitter for MockEventEmitter {
    fn emit(&self, event: &str, payload: Value) -> Result<(), String> {
        self.events
            .lock()
            .unwrap()
            .push((event.to_string(), payload));
        Ok(())
    }
}

type PendingResponseSender = oneshot::Sender<Result<JsonRpcResponse, BridgeError>>;
type RequestMessage = (JsonRpcRequest, PendingResponseSender);
type PendingRequestsMap = Arc<Mutex<HashMap<u64, PendingResponseSender>>>;

/// UNIX domain socket IPC bridge connecting the GUI to `omnilink-daemon`.
pub struct IpcBridge {
    socket_path: PathBuf,
    next_id: AtomicU64,
    emitter: Arc<dyn EventEmitter>,
    req_tx: Arc<Mutex<Option<mpsc::Sender<RequestMessage>>>>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
}

impl IpcBridge {
    /// Constructs a new IPC bridge instance.
    ///
    /// If `socket_path` is `None`, uses `omnilink_daemon::ipc::default_socket_path()`.
    pub fn new(socket_path: Option<PathBuf>, emitter: Arc<dyn EventEmitter>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            socket_path: socket_path.unwrap_or_else(default_socket_path),
            next_id: AtomicU64::new(1),
            emitter,
            req_tx: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Spawns the background connection and event listening task.
    ///
    /// Automatically attempts connection to the daemon socket and reconnects
    /// if the connection breaks or the daemon starts after the GUI.
    pub fn start(&self) {
        let socket_path = self.socket_path.clone();
        let emitter = self.emitter.clone();
        let req_tx_holder = self.req_tx.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            let retry_interval = Duration::from_millis(500);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match UnixStream::connect(&socket_path).await {
                    Ok(stream) => {
                        info!(path = %socket_path.display(), "Connected to Omnilink daemon IPC socket");
                        let (req_tx, req_rx) = mpsc::channel(32);
                        {
                            let mut lock = req_tx_holder.lock().await;
                            *lock = Some(req_tx);
                        }

                        // Run the active session until disconnect or shutdown
                        let session_res = run_ipc_session(
                            stream,
                            req_rx,
                            emitter.clone(),
                            &mut shutdown_rx,
                        )
                        .await;

                        {
                            let mut lock = req_tx_holder.lock().await;
                            *lock = None;
                        }

                        if let Err(e) = session_res {
                            debug!(error = %e, "IPC session ended");
                        }

                        // Notify frontend that daemon connection dropped
                        let _ = emitter.emit(
                            "device.status_changed",
                            serde_json::json!({ "connected": false }),
                        );
                    }
                    Err(e) => {
                        debug!(
                            path = %socket_path.display(),
                            error = %e,
                            "Could not connect to daemon socket, will retry..."
                        );
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(retry_interval) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Signals the background connection loop to shut down cleanly.
    pub fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Returns the target socket path configured for this bridge.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Checks whether the bridge currently has an active connection to the daemon.
    pub async fn is_connected(&self) -> bool {
        self.req_tx.lock().await.is_some()
    }

    /// Sends a JSON-RPC 2.0 request over the IPC socket and awaits the response.
    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, BridgeError> {
        let sender = {
            let lock = self.req_tx.lock().await;
            lock.clone().ok_or(BridgeError::NotConnected)?
        };

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(serde_json::json!(id)),
            method: method.to_string(),
            params,
        };

        let (resp_tx, resp_rx) = oneshot::channel();
        sender
            .send((req, resp_tx))
            .await
            .map_err(|_| BridgeError::NotConnected)?;

        match tokio::time::timeout(Duration::from_secs(5), resp_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(BridgeError::ChannelClosed),
            Err(_) => Err(BridgeError::Timeout),
        }
    }

    /// Queries the daemon for current pairing and connection telemetry (`status.get`).
    pub async fn get_status(&self) -> Result<StatusGetResult, BridgeError> {
        let resp = self
            .request("status.get", Some(serde_json::json!({})))
            .await?;
        if let Some(err) = resp.error {
            return Err(BridgeError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        let result_val = resp.result.unwrap_or(Value::Null);
        let status: StatusGetResult = serde_json::from_value(result_val)?;
        Ok(status)
    }

    /// Toggles remote mobile hotspot state (`device.toggle_hotspot`).
    pub async fn toggle_hotspot(&self, enable: bool) -> Result<bool, BridgeError> {
        let resp = self
            .request(
                "device.toggle_hotspot",
                Some(serde_json::json!({ "enable": enable })),
            )
            .await?;
        if let Some(err) = resp.error {
            return Err(BridgeError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        Ok(true)
    }

    /// Sends command to ring the phone at max volume (`device.ring_phone`).
    pub async fn ring_phone(&self) -> Result<bool, BridgeError> {
        let resp = self
            .request("device.ring_phone", Some(serde_json::json!({})))
            .await?;
        if let Some(err) = resp.error {
            return Err(BridgeError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        Ok(true)
    }
}

impl Drop for IpcBridge {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs a single active connection session to the daemon IPC socket.
async fn run_ipc_session(
    stream: UnixStream,
    mut req_rx: mpsc::Receiver<RequestMessage>,
    emitter: Arc<dyn EventEmitter>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(), BridgeError> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let pending_requests: PendingRequestsMap = Arc::new(Mutex::new(HashMap::new()));

    loop {
        tokio::select! {
            // Outbound request from GUI invoke commands
            req_item = req_rx.recv() => {
                match req_item {
                    Some((req, resp_tx)) => {
                        let id = match req.id.as_ref().and_then(|v| v.as_u64()) {
                            Some(id) => id,
                            None => {
                                let _ = resp_tx.send(Err(BridgeError::Rpc {
                                    code: -32600,
                                    message: "Missing numeric request id".to_string(),
                                }));
                                continue;
                            }
                        };

                        let json_line = match serde_json::to_string(&req) {
                            Ok(j) => format!("{j}\n"),
                            Err(e) => {
                                let _ = resp_tx.send(Err(BridgeError::Serialization(e)));
                                continue;
                            }
                        };

                        {
                            let mut map = pending_requests.lock().await;
                            map.insert(id, resp_tx);
                        }

                        if let Err(e) = writer.write_all(json_line.as_bytes()).await {
                            let mut map = pending_requests.lock().await;
                            if let Some(tx) = map.remove(&id) {
                                let _ = tx.send(Err(BridgeError::Io(e)));
                            }
                            return Err(BridgeError::NotConnected);
                        }
                        if let Err(e) = writer.flush().await {
                            let mut map = pending_requests.lock().await;
                            if let Some(tx) = map.remove(&id) {
                                let _ = tx.send(Err(BridgeError::Io(e)));
                            }
                            return Err(BridgeError::NotConnected);
                        }
                    }
                    None => {
                        // Request channel closed
                        break;
                    }
                }
            }

            // Inbound line from daemon socket
            line_res = lines.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }

                        let parsed: Value = match serde_json::from_str(trimmed) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(error = %e, "Invalid JSON received on IPC socket: {}", trimmed);
                                continue;
                            }
                        };

                        // 1. Check if broadcast event
                        if let Some(event_val) = parsed.get("event").and_then(|e| e.as_str()) {
                            let data = parsed.get("data").cloned().unwrap_or(Value::Null);
                            let _ = emitter.emit(event_val, data);
                            continue;
                        }

                        // 2. Check if JSON-RPC response
                        if let Some(id_val) = parsed.get("id") {
                            if let Some(id) = id_val.as_u64() {
                                let mut map = pending_requests.lock().await;
                                if let Some(tx) = map.remove(&id) {
                                    match serde_json::from_value::<JsonRpcResponse>(parsed) {
                                        Ok(resp) => {
                                            let _ = tx.send(Ok(resp));
                                        }
                                        Err(e) => {
                                            let _ = tx.send(Err(BridgeError::Serialization(e)));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF - server disconnected
                        debug!("IPC socket connection closed by daemon");
                        break;
                    }
                    Err(e) => {
                        debug!(error = %e, "IPC socket read error");
                        break;
                    }
                }
            }

            // Shutdown signal triggered
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    // Fail any remaining pending requests
    let mut map = pending_requests.lock().await;
    for (_, tx) in map.drain() {
        let _ = tx.send(Err(BridgeError::NotConnected));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bridge_not_connected_errors() {
        let emitter = Arc::new(MockEventEmitter::new());
        let bridge = IpcBridge::new(Some(PathBuf::from("/tmp/nonexistent.sock")), emitter);

        assert!(!bridge.is_connected().await);
        let err = bridge.get_status().await.unwrap_err();
        assert!(matches!(err, BridgeError::NotConnected));

        let err = bridge.toggle_hotspot(true).await.unwrap_err();
        assert!(matches!(err, BridgeError::NotConnected));

        let err = bridge.ring_phone().await.unwrap_err();
        assert!(matches!(err, BridgeError::NotConnected));
    }

    #[test]
    fn test_bridge_socket_path_default_and_custom() {
        let emitter = Arc::new(MockEventEmitter::new());
        let custom = PathBuf::from("/custom/omnilink.sock");
        let bridge = IpcBridge::new(Some(custom.clone()), emitter.clone());
        assert_eq!(bridge.socket_path(), custom.as_path());

        let bridge_default = IpcBridge::new(None, emitter);
        assert_eq!(bridge_default.socket_path(), default_socket_path().as_path());
    }

    #[test]
    fn test_mock_event_emitter_records_events() {
        let emitter = MockEventEmitter::new();
        assert!(emitter.recorded_events().is_empty());

        emitter
            .emit("test.event", serde_json::json!({ "foo": "bar" }))
            .unwrap();
        let events = emitter.recorded_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "test.event");
        assert_eq!(events[0].1["foo"], "bar");
    }
}
