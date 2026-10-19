//! Omnilink desktop GUI library and Tauri v2 application launcher.

pub mod bridge;
pub mod commands;

use std::sync::Arc;
use tauri::Manager;
use crate::bridge::{IpcBridge, TauriEventEmitter};
use crate::commands::{get_status, ring_phone, toggle_hotspot};

/// Initializes and launches the Tauri desktop application.
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let emitter = Arc::new(TauriEventEmitter::new(app_handle));
            let bridge = Arc::new(IpcBridge::new(None, emitter));
            bridge.start();
            app.manage(bridge);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            toggle_hotspot,
            ring_phone
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
