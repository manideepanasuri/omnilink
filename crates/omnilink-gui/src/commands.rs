//! Tauri IPC invoke commands exposed to the frontend.

use std::sync::Arc;
use omnilink_daemon::ipc::StatusGetResult;
use crate::bridge::IpcBridge;

/// Queries `status.get` via the daemon IPC socket.
#[tauri::command]
pub async fn get_status(
    bridge: tauri::State<'_, Arc<IpcBridge>>,
) -> Result<StatusGetResult, String> {
    bridge.get_status().await.map_err(|e| e.to_string())
}

/// Sends `device.toggle_hotspot` via the daemon IPC socket.
#[tauri::command]
pub async fn toggle_hotspot(
    enable: bool,
    bridge: tauri::State<'_, Arc<IpcBridge>>,
) -> Result<bool, String> {
    bridge.toggle_hotspot(enable).await.map_err(|e| e.to_string())
}

/// Sends command to ring the phone via the daemon IPC socket.
#[tauri::command]
pub async fn ring_phone(
    bridge: tauri::State<'_, Arc<IpcBridge>>,
) -> Result<bool, String> {
    bridge.ring_phone().await.map_err(|e| e.to_string())
}
