//! UNIX domain socket JSON-RPC 2.0 IPC server.
//!
//! Provides the local IPC interface connecting `omnilink-daemon` to `omnilink-cli`,
//! `omnilink-gui` (Tauri), and Waybar via `/run/user/<UID>/omnilink.sock`
//! (falling back to `/tmp/omnilink-<UID>.sock`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};
use serde::{Deserialize, Serialize};

use omnilink_protocol::packet::{CommandAction, ControlPacket};
use omnilink_protocol::types::DeviceStatus;
use crate::state::DaemonState;

/// Maximum allowable line length in bytes for incoming IPC JSON requests (64 KB).
pub const MAX_IPC_LINE_LEN: usize = 64 * 1024;

/// Errors that can occur during IPC server operations.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// Underlying I/O error on UNIX domain socket or file system.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization error.
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Socket path is already bound by an active listening server.
    #[error("Socket path already in use by active server: {0}")]
    AddressInUse(PathBuf),

    /// The IPC server has shut down.
    #[error("Server shut down")]
    Shutdown,

    /// Communication channel to client was closed.
    #[error("Client communication channel closed")]
    ChannelClosed,
}

/// Returns the default UNIX domain socket path for the current user.
///
/// Follows the path calculation:
/// Uses `/run/user/<UID>/omnilink.sock` if `/run/user/<UID>` exists and is accessible.
/// Falls back to `/tmp/omnilink-<UID>.sock` otherwise.
pub fn default_socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    socket_path_for_uid(uid)
}

/// Returns the socket path for a specific UID, checking whether `/run/user/<uid>` is accessible.
pub fn socket_path_for_uid(uid: u32) -> PathBuf {
    let run_user_dir = PathBuf::from(format!("/run/user/{}", uid));
    if is_directory_accessible(&run_user_dir) {
        run_user_dir.join("omnilink.sock")
    } else {
        PathBuf::from(format!("/tmp/omnilink-{}.sock", uid))
    }
}

/// Calculates the target socket path, respecting an optional override path.
pub fn calculate_socket_path(override_path: Option<&Path>) -> PathBuf {
    if let Some(path) = override_path {
        path.to_path_buf()
    } else {
        default_socket_path()
    }
}

/// Checks whether a directory exists and is accessible for reading, writing, and executing.
fn is_directory_accessible(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    use std::os::unix::ffi::OsStrExt;
    if let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) {
        unsafe { libc::access(c_path.as_ptr(), libc::R_OK | libc::W_OK | libc::X_OK) == 0 }
    } else {
        false
    }
}

/// Incoming JSON-RPC 2.0 request or notification from a socket client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version string (optional for flexibility, typically "2.0").
    #[serde(default)]
    pub jsonrpc: Option<String>,
    /// Unique request identifier (number, string, or null).
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    /// Method name to invoke.
    pub method: String,
    /// Method parameters (object, array, or null).
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Outgoing JSON-RPC 2.0 response sent to a socket client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version string, always "2.0".
    pub jsonrpc: String,
    /// Request identifier matching the incoming request, or null if unidentifiable.
    pub id: serde_json::Value,
    /// Successful result payload, if request succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error payload, if request failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code (standard JSON-RPC or application-specific).
    pub code: i32,
    /// Short human-readable error description.
    pub message: String,
    /// Optional structured error data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    /// Constructs a successful JSON-RPC 2.0 response.
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            result: Some(result),
            error: None,
        }
    }

    /// Constructs an error JSON-RPC 2.0 response.
    pub fn error(id: Option<serde_json::Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: id.unwrap_or(serde_json::Value::Null),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    /// JSON parse error (-32700).
    pub fn invalid_json() -> Self {
        Self::error(None, -32700, "Parse error")
    }

    /// Invalid request error (-32600).
    pub fn invalid_request(id: Option<serde_json::Value>) -> Self {
        Self::error(id, -32600, "Invalid Request")
    }

    /// Method not found error (-32601).
    pub fn method_not_found(id: Option<serde_json::Value>, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }

    /// Invalid method parameters error (-32602).
    pub fn invalid_params(id: Option<serde_json::Value>, details: &str) -> Self {
        Self::error(id, -32602, format!("Invalid params: {details}"))
    }

    /// Internal error (-32603).
    pub fn internal_error(id: Option<serde_json::Value>, msg: &str) -> Self {
        Self::error(id, -32603, format!("Internal error: {msg}"))
    }

    /// Companion device not connected error (-32000).
    pub fn device_not_connected(id: Option<serde_json::Value>) -> Self {
        Self::error(id, -32000, "Companion device not connected")
    }
}

/// Asynchronous broadcast event streamed from daemon to connected socket clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcEvent {
    /// Event name identifier (e.g. "device.status_changed", "clipboard.updated").
    pub event: String,
    /// Event data payload.
    pub data: serde_json::Value,
}

impl IpcEvent {
    /// Creates a generic IPC broadcast event.
    pub fn new(event: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }

    /// Creates a `device.status_changed` event from a `DeviceStatus`.
    pub fn device_status_changed(status: &DeviceStatus) -> Self {
        Self {
            event: "device.status_changed".to_string(),
            data: serde_json::to_value(status).unwrap_or_default(),
        }
    }

    /// Creates a `clipboard.updated` event from a MIME type and preview string.
    pub fn clipboard_updated(mime_type: impl Into<String>, preview: impl Into<String>) -> Self {
        Self {
            event: "clipboard.updated".to_string(),
            data: serde_json::json!({
                "mime_type": mime_type.into(),
                "preview": preview.into(),
            }),
        }
    }
}

/// Telemetry and connection status payload returned by `status.get`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusGetResult {
    /// Whether an active companion session is established.
    pub connected: bool,
    /// Paired or connected device name, if available.
    pub device_name: Option<String>,
    /// Device battery charge percentage (0..100).
    pub battery_pct: Option<u8>,
    /// Whether the device is charging.
    pub is_charging: Option<bool>,
    /// Connected Wi-Fi SSID name, if available.
    pub wifi_ssid: Option<String>,
    /// Cellular signal strength in bars (0..4).
    pub cellular_signal: Option<u8>,
    /// Whether mobile hotspot is currently active.
    pub hotspot_active: Option<bool>,
}

/// Dispatches an incoming raw JSON line request against daemon state.
pub async fn handle_ipc_request(line: &str, state: &Arc<DaemonState>) -> JsonRpcResponse {
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return JsonRpcResponse::invalid_json(),
    };

    let obj = match parsed.as_object() {
        Some(o) => o,
        None => return JsonRpcResponse::invalid_request(None),
    };

    let id = obj.get("id").cloned();
    let method = match obj.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => return JsonRpcResponse::invalid_request(id),
    };

    let params = obj.get("params").cloned().unwrap_or(serde_json::Value::Null);

    match method {
        "ping" => JsonRpcResponse::success(id, serde_json::Value::String("pong".to_string())),
        "status.get" => {
            if !params.is_null() && !params.is_object() {
                return JsonRpcResponse::invalid_params(id, "params must be an object or omitted");
            }
            let session_lock = state.session.read().await;
            let paired_lock = state.paired_device.read().await;

            let connected = session_lock.is_some();
            let device_name = paired_lock
                .as_ref()
                .map(|p| p.name.clone())
                .or_else(|| session_lock.as_ref().map(|s| s.device_id.clone()));

            let (battery_pct, is_charging, wifi_ssid, cellular_signal, hotspot_active) =
                if let Some(ref session) = *session_lock {
                    (
                        Some(session.battery_pct),
                        Some(session.is_charging),
                        session.wifi_ssid.clone(),
                        Some(session.cellular_signal),
                        Some(session.hotspot_active),
                    )
                } else {
                    (None, None, None, None, None)
                };

            let result = StatusGetResult {
                connected,
                device_name,
                battery_pct,
                is_charging,
                wifi_ssid,
                cellular_signal,
                hotspot_active,
            };

            JsonRpcResponse::success(id, serde_json::to_value(result).unwrap_or_default())
        }
        "device.toggle_hotspot" => {
            if !params.is_null() && !params.is_object() {
                return JsonRpcResponse::invalid_params(id, "params must be an object or omitted");
            }
            match state
                .send_command(ControlPacket::Command {
                    action: CommandAction::ToggleHotspot,
                })
                .await
            {
                Ok(()) => JsonRpcResponse::success(id, serde_json::json!({ "success": true })),
                Err(_) => JsonRpcResponse::device_not_connected(id),
            }
        }
        "device.ring_phone" => {
            if !params.is_null() && !params.is_object() {
                return JsonRpcResponse::invalid_params(id, "params must be an object or omitted");
            }
            match state
                .send_command(ControlPacket::Command {
                    action: CommandAction::RingPhone,
                })
                .await
            {
                Ok(()) => JsonRpcResponse::success(id, serde_json::json!({ "success": true })),
                Err(_) => JsonRpcResponse::device_not_connected(id),
            }
        }
        "notification.reply" => {
            #[derive(Deserialize)]
            struct ReplyParams {
                notification_id: String,
                #[serde(default = "default_inline_action_key")]
                action_key: String,
                reply_text: String,
            }
            fn default_inline_action_key() -> String {
                "inline-reply".to_string()
            }

            let p: ReplyParams = match serde_json::from_value(params) {
                Ok(val) => val,
                Err(e) => return JsonRpcResponse::invalid_params(id, &e.to_string()),
            };

            match state
                .send_notification_reply(p.notification_id, p.action_key, p.reply_text)
                .await
            {
                Ok(()) => JsonRpcResponse::success(id, serde_json::json!({ "success": true })),
                Err(_) => JsonRpcResponse::device_not_connected(id),
            }
        }
        "notification.dismiss" => {
            #[derive(Deserialize)]
            struct DismissParams {
                #[serde(alias = "notification_id")]
                id: String,
            }

            let p: DismissParams = match serde_json::from_value(params) {
                Ok(val) => val,
                Err(e) => return JsonRpcResponse::invalid_params(id, &e.to_string()),
            };

            match state.send_notification_dismiss(p.id).await {
                Ok(()) => JsonRpcResponse::success(id, serde_json::json!({ "success": true })),
                Err(_) => JsonRpcResponse::device_not_connected(id),
            }
        }
        unknown => JsonRpcResponse::method_not_found(id, unknown),
    }
}

/// Handles an active client connection on a UNIX domain socket.
async fn handle_client_connection(
    stream: UnixStream,
    state: Arc<DaemonState>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let (reader, mut writer) = stream.into_split();
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(64);

    // Outbound writer task
    let writer_task = tokio::spawn(async move {
        while let Some(line) = outbound_rx.recv().await {
            if let Err(e) = writer.write_all(line.as_bytes()).await {
                debug!(error = %e, "IPC client write failed (likely disconnected)");
                break;
            }
            if let Err(e) = writer.flush().await {
                debug!(error = %e, "IPC client flush failed");
                break;
            }
        }
    });

    // Broadcast event forwarding task
    let mut event_rx = state.subscribe_ipc_events();
    let outbound_tx_events = outbound_tx.clone();
    let mut shutdown_rx_events = shutdown_rx.clone();
    let event_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                res = event_rx.recv() => {
                    match res {
                        Ok(event) => {
                            if let Ok(json) = serde_json::to_string(&event) {
                                let line = format!("{json}\n");
                                if outbound_tx_events.send(line).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "IPC client lagged behind broadcast events");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
                _ = shutdown_rx_events.changed() => {
                    if *shutdown_rx_events.borrow() {
                        break;
                    }
                }
            }
        }
    });

    // Inbound reader loop
    let mut lines = BufReader::new(reader).lines();
    loop {
        tokio::select! {
            line_res = lines.next_line() => {
                match line_res {
                    Ok(Some(line)) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if trimmed.len() > MAX_IPC_LINE_LEN {
                            let response = JsonRpcResponse::invalid_request(None);
                            if let Ok(json) = serde_json::to_string(&response) {
                                let resp_line = format!("{json}\n");
                                if outbound_tx.send(resp_line).await.is_err() {
                                    break;
                                }
                            }
                            continue;
                        }
                        let response = handle_ipc_request(trimmed, &state).await;
                        if let Ok(json) = serde_json::to_string(&response) {
                            let resp_line = format!("{json}\n");
                            if outbound_tx.send(resp_line).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        debug!("IPC client disconnected cleanly (EOF)");
                        break;
                    }
                    Err(e) => {
                        debug!(error = %e, "IPC client read error");
                        break;
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
        }
    }

    event_task.abort();
    let _ = event_task.await;
    drop(outbound_tx);
    let _ = writer_task.await;
}

/// Handle to a running IPC server.
///
/// Ensures graceful shutdown and unlinks the socket file on drop or when `stop()` is called.
pub struct IpcHandle {
    socket_path: PathBuf,
    shutdown_tx: Option<watch::Sender<bool>>,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for IpcHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcHandle")
            .field("socket_path", &self.socket_path)
            .finish()
    }
}

impl IpcHandle {
    /// Returns the socket path this server is bound to.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Stops the IPC server gracefully and unlinks the socket file.
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.await;
        }
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

impl Drop for IpcHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

/// UNIX domain socket JSON-RPC IPC Server.
pub struct IpcServer;

impl IpcServer {
    /// Binds the IPC server to the calculated or overridden socket path.
    pub async fn bind(
        override_path: Option<&Path>,
        state: Arc<DaemonState>,
    ) -> Result<IpcHandle, IpcError> {
        let socket_path = calculate_socket_path(override_path);
        Self::bind_path(socket_path, state).await
    }

    /// Binds the IPC server to an explicit socket path.
    pub async fn bind_path(
        socket_path: PathBuf,
        state: Arc<DaemonState>,
    ) -> Result<IpcHandle, IpcError> {
        if let Some(parent) = socket_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        if socket_path.exists() {
            // Check if another daemon is actively listening on this socket
            if UnixStream::connect(&socket_path).await.is_ok() {
                return Err(IpcError::AddressInUse(socket_path));
            }
            // Stale socket, clean it up before re-binding
            let _ = std::fs::remove_file(&socket_path);
        }

        let listener = UnixListener::bind(&socket_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        }

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let path_clone = socket_path.clone();

        let join_handle = tokio::spawn(async move {
            info!(path = %path_clone.display(), "IPC server listener started");
            let mut shutdown_rx_loop = shutdown_rx.clone();

            loop {
                tokio::select! {
                    _ = shutdown_rx_loop.changed() => {
                        if *shutdown_rx_loop.borrow() {
                            info!("IPC server shutting down accept loop");
                            break;
                        }
                    }
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, _addr)) => {
                                let client_state = state.clone();
                                let client_shutdown = shutdown_rx.clone();
                                tokio::spawn(async move {
                                    handle_client_connection(stream, client_state, client_shutdown).await;
                                });
                            }
                            Err(e) => {
                                warn!(error = %e, "IPC listener accept error");
                            }
                        }
                    }
                }
            }

            if path_clone.exists() {
                let _ = std::fs::remove_file(&path_clone);
            }
        });

        Ok(IpcHandle {
            socket_path,
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
        })
    }
}

/// Helper client for interacting with the Omnilink daemon IPC server.
pub struct IpcClient {
    lines: tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl IpcClient {
    /// Connects to the IPC server at the specified socket path.
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path).await?;
        let (read_half, write_half) = stream.into_split();
        let lines = BufReader::new(read_half).lines();
        Ok(Self {
            lines,
            writer: write_half,
        })
    }

    /// Sends a raw string line over the socket (appends `\n` if missing).
    pub async fn send_raw(&mut self, data: &str) -> Result<(), IpcError> {
        let mut msg = data.to_string();
        if !msg.ends_with('\n') {
            msg.push('\n');
        }
        self.writer.write_all(msg.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }

    /// Sends a JSON-RPC request and returns the corresponding response.
    ///
    /// Skips any broadcast events received while awaiting the RPC response.
    pub async fn request(&mut self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, IpcError> {
        let json = serde_json::to_string(request)?;
        self.send_raw(&json).await?;
        while let Some(line) = self.next_line().await? {
            let val: serde_json::Value = serde_json::from_str(&line)?;
            if val.get("event").is_some() {
                continue;
            }
            let resp: JsonRpcResponse = serde_json::from_value(val)?;
            return Ok(resp);
        }
        Err(IpcError::ChannelClosed)
    }

    /// Reads the next newline-delimited line from the daemon.
    pub async fn next_line(&mut self) -> Result<Option<String>, IpcError> {
        self.lines.next_line().await.map_err(IpcError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_path_calculation_fallback() {
        let path = socket_path_for_uid(99999999);
        assert_eq!(path, PathBuf::from("/tmp/omnilink-99999999.sock"));
    }

    #[test]
    fn test_path_calculation_override() {
        let custom = Path::new("/custom/socket.sock");
        let path = calculate_socket_path(Some(custom));
        assert_eq!(path, custom);
    }

    #[tokio::test]
    async fn test_rpc_request_ping() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let state = Arc::new(DaemonState::new(signing_key, None));

        let response = handle_ipc_request(r#"{"id":10,"method":"ping"}"#, &state).await;
        assert_eq!(response.id, serde_json::json!(10));
        assert_eq!(response.result, Some(serde_json::json!("pong")));
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_rpc_request_invalid_json() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let state = Arc::new(DaemonState::new(signing_key, None));

        let response = handle_ipc_request("not valid json", &state).await;
        assert_eq!(response.id, serde_json::Value::Null);
        assert!(response.result.is_none());
        let err = response.error.expect("Expected error");
        assert_eq!(err.code, -32700);
        assert_eq!(err.message, "Parse error");
    }

    #[tokio::test]
    async fn test_rpc_request_invalid_method() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let state = Arc::new(DaemonState::new(signing_key, None));

        let response = handle_ipc_request(r#"{"id":1,"method":"foo.bar"}"#, &state).await;
        assert_eq!(response.id, serde_json::json!(1));
        let err = response.error.expect("Expected error");
        assert_eq!(err.code, -32601);
    }

    #[tokio::test]
    async fn test_rpc_request_invalid_params() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let state = Arc::new(DaemonState::new(signing_key, None));

        let response = handle_ipc_request(
            r#"{"id":2,"method":"notification.reply","params":"not_an_object"}"#,
            &state,
        )
        .await;
        assert_eq!(response.id, serde_json::json!(2));
        let err = response.error.expect("Expected error");
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn test_ipc_event_serialization() {
        let event = IpcEvent::clipboard_updated("text/plain", "https://omarchy.org");
        let json = serde_json::to_string(&event).expect("serialize event");
        assert!(json.contains(r#""event":"clipboard.updated""#));
        assert!(json.contains(r#""mime_type":"text/plain""#));
        assert!(json.contains(r#""preview":"https://omarchy.org""#));
    }
}
