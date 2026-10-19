//! Integration tests for omnilink-gui UNIX Domain Socket IPC Bridge.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use omnilink_daemon::ipc::StatusGetResult;
use omnilink_gui::bridge::{BridgeError, IpcBridge, MockEventEmitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
static TEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Helper to generate a unique temporary socket path.
fn temp_socket_path() -> PathBuf {
    let pid = std::process::id();
    let count = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(format!("/tmp/omnilink-gui-test-{}-{}-{}.sock", pid, count, time))
}

#[tokio::test]
async fn test_bridge_get_status_success() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter));
    bridge.start();

    // Mock daemon server connection handling
    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            assert_eq!(req["method"], "status.get");
            let id = &req["id"];

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "connected": true,
                    "device_name": "Galaxy S24 Ultra",
                    "battery_pct": 82,
                    "is_charging": false,
                    "wifi_ssid": "Home-5GHz",
                    "cellular_signal": 4,
                    "hotspot_active": false
                }
            });
            let line = format!("{}\n", resp);
            writer.write_all(line.as_bytes()).await.expect("write response");
            writer.flush().await.expect("flush");
        }
    });

    // Wait until bridge connects
    tokio::time::sleep(Duration::from_millis(100)).await;

    let status = bridge.get_status().await.expect("get_status");
    assert_eq!(
        status,
        StatusGetResult {
            connected: true,
            device_name: Some("Galaxy S24 Ultra".to_string()),
            battery_pct: Some(82),
            is_charging: Some(false),
            wifi_ssid: Some("Home-5GHz".to_string()),
            cellular_signal: Some(4),
            hotspot_active: Some(false),
        }
    );

    bridge.stop();
    let _ = server_task.await;
    let _ = std::fs::remove_file(&socket_path);
}

#[tokio::test]
async fn test_bridge_toggle_hotspot_command() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter));
    bridge.start();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            assert_eq!(req["method"], "device.toggle_hotspot");
            assert_eq!(req["params"]["enable"], true);
            let id = &req["id"];

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "success": true }
            });
            let line = format!("{}\n", resp);
            writer.write_all(line.as_bytes()).await.expect("write response");
            writer.flush().await.expect("flush");
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = bridge.toggle_hotspot(true).await.expect("toggle_hotspot");
    assert!(result);

    bridge.stop();
    let _ = server_task.await;
    let _ = std::fs::remove_file(&socket_path);
}

#[tokio::test]
async fn test_bridge_ring_phone_command() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter));
    bridge.start();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            assert_eq!(req["method"], "device.ring_phone");
            let id = &req["id"];

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "success": true }
            });
            let line = format!("{}\n", resp);
            writer.write_all(line.as_bytes()).await.expect("write response");
            writer.flush().await.expect("flush");
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = bridge.ring_phone().await.expect("ring_phone");
    assert!(result);

    bridge.stop();
    let _ = server_task.await;
    let _ = std::fs::remove_file(&socket_path);
}

#[tokio::test]
async fn test_bridge_broadcast_event_forwarding() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter.clone()));
    bridge.start();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let (_, mut writer) = stream.into_split();

        // Send two broadcast events
        let event1 = serde_json::json!({
            "event": "device.status_changed",
            "data": { "battery_pct": 91, "is_charging": true }
        });
        writer
            .write_all(format!("{}\n", event1).as_bytes())
            .await
            .expect("write event1");
        writer.flush().await.expect("flush 1");

        let event2 = serde_json::json!({
            "event": "clipboard.updated",
            "data": { "mime_type": "text/plain", "preview": "https://omarchy.org" }
        });
        writer
            .write_all(format!("{}\n", event2).as_bytes())
            .await
            .expect("write event2");
        writer.flush().await.expect("flush 2");

        // Keep connection open briefly so client can receive
        tokio::time::sleep(Duration::from_millis(200)).await;
    });

    // Wait for events to be processed
    tokio::time::sleep(Duration::from_millis(300)).await;

    let recorded = emitter.recorded_events();
    assert!(
        recorded.iter().any(|(ev, data)| {
            ev == "device.status_changed"
                && data.get("battery_pct").and_then(|v| v.as_u64()) == Some(91)
        }),
        "Expected device.status_changed event with battery_pct 91"
    );
    assert!(
        recorded.iter().any(|(ev, data)| {
            ev == "clipboard.updated"
                && data.get("preview").and_then(|v| v.as_str()) == Some("https://omarchy.org")
        }),
        "Expected clipboard.updated event"
    );

    bridge.stop();
    let _ = server_task.await;
    let _ = std::fs::remove_file(&socket_path);
}

#[tokio::test]
async fn test_bridge_reconnection_and_resilience() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind first mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter.clone()));
    bridge.start();

    // 1. First server session: answer status.get then drop
    let server1_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept on server 1");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            let id = &req["id"];
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "connected": true,
                    "device_name": "Galaxy S24 Ultra",
                    "battery_pct": 50,
                    "is_charging": false,
                    "wifi_ssid": "Test-Wi-Fi",
                    "cellular_signal": 3,
                    "hotspot_active": false
                }
            });
            writer
                .write_all(format!("{}\n", resp).as_bytes())
                .await
                .expect("write");
            writer.flush().await.expect("flush");
        }
        // Drop stream and listener
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let status1 = bridge.get_status().await.expect("first get_status");
    assert_eq!(status1.battery_pct, Some(50));

    // Wait for server1 to exit
    let _ = server1_task.await;
    let _ = std::fs::remove_file(&socket_path);

    // Wait a brief period for bridge to detect disconnect
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 2. Start second server on same socket path
    let listener2 = UnixListener::bind(&socket_path).expect("bind second mock server");
    let server2_task = tokio::spawn(async move {
        let (stream, _) = listener2.accept().await.expect("accept on server 2");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            let id = &req["id"];
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "connected": true,
                    "device_name": "Galaxy S24 Ultra",
                    "battery_pct": 99,
                    "is_charging": true,
                    "wifi_ssid": "Home-5GHz",
                    "cellular_signal": 4,
                    "hotspot_active": true
                }
            });
            writer
                .write_all(format!("{}\n", resp).as_bytes())
                .await
                .expect("write");
            writer.flush().await.expect("flush");
        }
    });

    // Wait for bridge reconnection (retry interval is 500ms)
    tokio::time::sleep(Duration::from_millis(800)).await;

    let status2 = bridge.get_status().await.expect("reconnected get_status");
    assert_eq!(status2.battery_pct, Some(99));
    assert_eq!(status2.hotspot_active, Some(true));

    bridge.stop();
    let _ = server2_task.await;
    let _ = std::fs::remove_file(&socket_path);
}

#[tokio::test]
async fn test_bridge_rpc_error_handling() {
    let socket_path = temp_socket_path();
    let listener = UnixListener::bind(&socket_path).expect("bind mock server");

    let emitter = Arc::new(MockEventEmitter::new());
    let bridge = Arc::new(IpcBridge::new(Some(socket_path.clone()), emitter));
    bridge.start();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept client");
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();

        if let Ok(Some(line)) = lines.next_line().await {
            let req: serde_json::Value = serde_json::from_str(&line).expect("parse request");
            let id = &req["id"];

            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32000,
                    "message": "Companion device not connected"
                }
            });
            writer
                .write_all(format!("{}\n", resp).as_bytes())
                .await
                .expect("write");
            writer.flush().await.expect("flush");
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let err = bridge
        .toggle_hotspot(true)
        .await
        .expect_err("should return RPC error");
    match err {
        BridgeError::Rpc { code, message } => {
            assert_eq!(code, -32000);
            assert_eq!(message, "Companion device not connected");
        }
        other => panic!("Unexpected error variant: {:?}", other),
    }

    bridge.stop();
    let _ = server_task.await;
    let _ = std::fs::remove_file(&socket_path);
}
