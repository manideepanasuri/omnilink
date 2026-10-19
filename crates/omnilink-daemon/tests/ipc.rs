//! UNIX Domain Socket JSON-RPC Server integration tests.
//!
//! Tests socket binding, path calculation, JSON-RPC 2.0 requests (status.get,
//! device.toggle_hotspot, notification.reply, notification.dismiss, ping),
//! broadcast events, client disconnect resilience, and cleanup.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use omnilink_daemon::ipc::{
    calculate_socket_path, default_socket_path, socket_path_for_uid, IpcClient, IpcError,
    IpcEvent, IpcServer, JsonRpcRequest, StatusGetResult,
};
use omnilink_daemon::quic::{
    init_crypto_provider, make_client_config, read_control_packet, read_notification_packet,
    QuicServer,
};
use omnilink_daemon::state::{DaemonState, PairedDevice};
use omnilink_protocol::packet::{
    CommandAction, ControlPacket, NotificationDismissPacket, NotificationReplyPacket,
    NotificationStreamPacket,
};
use omnilink_protocol::types::DeviceStatus;
use quinn::Endpoint;
use uuid::Uuid;

/// Helper to generate a unique temporary socket path under `/tmp`.
fn temp_socket_path() -> PathBuf {
    PathBuf::from(format!("/tmp/omnilink-test-{}.sock", Uuid::new_v4()))
}

/// Helper to set up a test daemon listening on an ephemeral QUIC port with a paired client key.
async fn setup_test_daemon() -> (QuicServer, Arc<DaemonState>, SigningKey, SigningKey) {
    init_crypto_provider();

    let mut csprng = rand::rngs::OsRng;
    let daemon_signing_key = SigningKey::generate(&mut csprng);
    let client_signing_key = SigningKey::generate(&mut csprng);

    let paired_device = PairedDevice {
        name: "Galaxy S24 Ultra".to_string(),
        public_key: client_signing_key.verifying_key(),
        paired_at: 1729250000,
    };

    let daemon_state = Arc::new(DaemonState::new(daemon_signing_key.clone(), Some(paired_device)));

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server = QuicServer::bind(addr, daemon_state.clone())
        .await
        .expect("Failed to bind QuicServer");

    (server, daemon_state, daemon_signing_key, client_signing_key)
}

/// Helper to create a client QUIC endpoint and connect to the daemon.
async fn connect_quic_client(
    server_addr: SocketAddr,
    client_signing_key: &SigningKey,
    daemon_signing_key: &SigningKey,
) -> quinn::Connection {
    let client_config = make_client_config(
        client_signing_key,
        &daemon_signing_key.verifying_key(),
        Some(Duration::from_secs(5)),
    )
    .expect("Failed to build client config");

    let mut client_endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap())
        .expect("Failed to create client endpoint");
    client_endpoint.set_default_client_config(client_config);

    client_endpoint
        .connect(server_addr, "localhost")
        .expect("Failed to initiate connect")
        .await
        .expect("Failed to establish QUIC connection")
}

#[test]
fn test_ipc_path_calculation_and_overrides() {
    // 1. Non-existent UID falls back to /tmp/omnilink-<UID>.sock
    let fallback = socket_path_for_uid(99999999);
    assert_eq!(fallback, PathBuf::from("/tmp/omnilink-99999999.sock"));

    // 2. Default socket path calculation for current user
    let default_path = default_socket_path();
    let uid = unsafe { libc::getuid() };
    let expected_run_dir = PathBuf::from(format!("/run/user/{}", uid));
    if expected_run_dir.is_dir() {
        assert_eq!(default_path, expected_run_dir.join("omnilink.sock"));
    } else {
        assert_eq!(default_path, PathBuf::from(format!("/tmp/omnilink-{}.sock", uid)));
    }

    // 3. Explicit override path is respected
    let custom = PathBuf::from("/tmp/my-custom-omnilink.sock");
    assert_eq!(calculate_socket_path(Some(&custom)), custom);
    assert_eq!(calculate_socket_path(None), default_path);
}

#[tokio::test]
async fn test_ipc_socket_binding_and_cleanup() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let handle = IpcServer::bind(Some(&path), state)
        .await
        .expect("Failed to bind IPC server");
    assert!(path.exists(), "Socket file should exist after bind");

    handle.stop().await;
    assert!(!path.exists(), "Socket file should be removed after stop");
}

#[tokio::test]
async fn test_ipc_cleanup_on_drop() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    {
        let _handle = IpcServer::bind(Some(&path), state)
            .await
            .expect("Failed to bind IPC server");
        assert!(path.exists(), "Socket file should exist after bind");
    }

    // After drop, file should be unlinked
    assert!(!path.exists(), "Socket file should be removed on drop");
}

#[tokio::test]
async fn test_ipc_stale_socket_recovery_and_active_detection() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    // Create an orphaned stale socket file
    std::fs::write(&path, b"stale socket").expect("Failed to write dummy file");
    assert!(path.exists());

    // Bind should detect no server is listening and overwrite the stale file
    let handle = IpcServer::bind(Some(&path), state.clone())
        .await
        .expect("Binding should succeed over stale file");
    assert!(path.exists());

    // Trying to bind a second server while the first is active must fail with AddressInUse
    let second_bind = IpcServer::bind(Some(&path), state).await;
    match second_bind {
        Err(IpcError::AddressInUse(conflict_path)) => assert_eq!(conflict_path, path),
        other => panic!("Expected AddressInUse error, got {other:?}"),
    }

    handle.stop().await;
    assert!(!path.exists());
}

#[tokio::test]
async fn test_ipc_ping_pong() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let handle = IpcServer::bind(Some(&path), state).await.expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    let req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(42)),
        method: "ping".to_string(),
        params: None,
    };

    let resp = client.request(&req).await.expect("request ping");
    assert_eq!(resp.id, serde_json::json!(42));
    assert_eq!(resp.result, Some(serde_json::json!("pong")));
    assert!(resp.error.is_none());

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_status_get_disconnected() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let paired_device = PairedDevice {
        name: "Galaxy S24 Ultra".to_string(),
        public_key: signing_key.verifying_key(),
        paired_at: 1729250000,
    };
    let state = Arc::new(DaemonState::new(signing_key, Some(paired_device)));

    let handle = IpcServer::bind(Some(&path), state).await.expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    let req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(1)),
        method: "status.get".to_string(),
        params: Some(serde_json::json!({})),
    };

    let resp = client.request(&req).await.expect("request status.get");
    assert_eq!(resp.id, serde_json::json!(1));
    assert!(resp.error.is_none());

    let status: StatusGetResult =
        serde_json::from_value(resp.result.unwrap()).expect("parse StatusGetResult");
    assert!(!status.connected);
    assert_eq!(status.device_name, Some("Galaxy S24 Ultra".to_string()));
    assert_eq!(status.battery_pct, None);
    assert_eq!(status.is_charging, None);
    assert_eq!(status.wifi_ssid, None);
    assert_eq!(status.cellular_signal, None);
    assert_eq!(status.hotspot_active, None);

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_status_get_connected() {
    let (quic_server, daemon_state, daemon_key, client_key) = setup_test_daemon().await;
    let quic_addr = quic_server.local_addr().unwrap();
    let _quic_handle = quic_server.start();
    let path = temp_socket_path();

    let ipc_handle = IpcServer::bind(Some(&path), daemon_state.clone())
        .await
        .expect("bind server");
    let _quic_conn =
        connect_quic_client(quic_addr, &client_key, &daemon_key).await;

    // Wait for QUIC session demuxer to populate daemon_state.session
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(daemon_state.is_connected().await);

    // Update session telemetry directly
    daemon_state
        .update_device_status(DeviceStatus {
            battery_pct: 82,
            is_charging: false,
            wifi_ssid: Some("Home-5GHz".to_string()),
            cellular_signal: 4,
            hotspot_active: false,
        })
        .await;

    let mut client = IpcClient::connect(&path).await.expect("connect client");

    let req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(1)),
        method: "status.get".to_string(),
        params: None,
    };

    let resp = client.request(&req).await.expect("request status.get");
    assert_eq!(resp.id, serde_json::json!(1));
    assert!(resp.error.is_none());

    let status: StatusGetResult =
        serde_json::from_value(resp.result.unwrap()).expect("parse StatusGetResult");
    assert!(status.connected);
    assert_eq!(status.device_name, Some("Galaxy S24 Ultra".to_string()));
    assert_eq!(status.battery_pct, Some(82));
    assert_eq!(status.is_charging, Some(false));
    assert_eq!(status.wifi_ssid, Some("Home-5GHz".to_string()));
    assert_eq!(status.cellular_signal, Some(4));
    assert_eq!(status.hotspot_active, Some(false));

    ipc_handle.stop().await;
}

#[tokio::test]
async fn test_ipc_device_toggle_hotspot_disconnected_and_connected() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let ipc_handle = IpcServer::bind(Some(&path), state.clone())
        .await
        .expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // 1. Disconnected test: returns device_not_connected (-32000)
    let req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(2)),
        method: "device.toggle_hotspot".to_string(),
        params: Some(serde_json::json!({ "enable": true })),
    };

    let resp = client.request(&req).await.expect("request hotspot disconnected");
    assert_eq!(resp.id, serde_json::json!(2));
    assert!(resp.result.is_none());
    let err = resp.error.expect("Expected error");
    assert_eq!(err.code, -32000);
    assert!(err.message.contains("not connected"));

    ipc_handle.stop().await;

    // 2. Connected test: commands are dispatched to the connected companion
    let (quic_server, daemon_state, daemon_key, client_key) = setup_test_daemon().await;
    let quic_addr = quic_server.local_addr().unwrap();
    let _quic_handle = quic_server.start();
    let path2 = temp_socket_path();
    let ipc_handle2 = IpcServer::bind(Some(&path2), daemon_state.clone())
        .await
        .expect("bind server");

    let quic_conn =
        connect_quic_client(quic_addr, &client_key, &daemon_key).await;
    let (mut client_send, mut client_recv) = quic_conn.open_bi().await.expect("open Stream 0");
    omnilink_daemon::quic::write_control_packet(&mut client_send, &ControlPacket::Ping { timestamp: 600 })
        .await
        .unwrap();
    let pong = read_control_packet(&mut client_recv).await.unwrap();
    assert_eq!(pong, ControlPacket::Pong { timestamp: 600 });

    let mut client2 = IpcClient::connect(&path2).await.expect("connect client 2");
    let req2 = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(20)),
        method: "device.toggle_hotspot".to_string(),
        params: Some(serde_json::json!({ "enable": true })),
    };

    let resp2 = client2.request(&req2).await.expect("request hotspot connected");
    assert_eq!(resp2.id, serde_json::json!(20));
    assert_eq!(resp2.result, Some(serde_json::json!({ "success": true })));

    // Verify companion client received the Command packet on Stream 0
    let packet = read_control_packet(&mut client_recv)
        .await
        .expect("read command on client stream");
    assert_eq!(
        packet,
        ControlPacket::Command {
            action: CommandAction::ToggleHotspot,
        }
    );

    let _ = client_send.finish();
    ipc_handle2.stop().await;
}

#[tokio::test]
async fn test_ipc_device_ring_phone() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let ipc_handle = IpcServer::bind(Some(&path), state.clone())
        .await
        .expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // 1. Disconnected -> error -32000
    let req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(101)),
        method: "device.ring_phone".to_string(),
        params: None,
    };
    let resp = client.request(&req).await.expect("request");
    assert_eq!(resp.error.unwrap().code, -32000);

    ipc_handle.stop().await;
}

#[tokio::test]
async fn test_ipc_notification_reply_and_dismiss() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let ipc_handle = IpcServer::bind(Some(&path), state.clone())
        .await
        .expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // 1. Disconnected reply
    let reply_req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(3)),
        method: "notification.reply".to_string(),
        params: Some(serde_json::json!({
            "notification_id": "0|com.whatsapp|1001",
            "reply_text": "Sounds good!"
        })),
    };
    let resp = client.request(&reply_req).await.expect("reply disconnected");
    assert_eq!(resp.error.unwrap().code, -32000);

    // 2. Disconnected dismiss
    let dismiss_req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(4)),
        method: "notification.dismiss".to_string(),
        params: Some(serde_json::json!({
            "notification_id": "0|com.whatsapp|1001"
        })),
    };
    let resp = client.request(&dismiss_req).await.expect("dismiss disconnected");
    assert_eq!(resp.error.unwrap().code, -32000);

    ipc_handle.stop().await;

    // 3. Connected reply and dismiss
    let (quic_server, daemon_state, daemon_key, client_key) = setup_test_daemon().await;
    let quic_addr = quic_server.local_addr().unwrap();
    let _quic_handle = quic_server.start();
    let path2 = temp_socket_path();
    let ipc_handle2 = IpcServer::bind(Some(&path2), daemon_state.clone())
        .await
        .expect("bind server");

    let quic_conn =
        connect_quic_client(quic_addr, &client_key, &daemon_key).await;
    // Open Stream 0
    let (mut s0_send, mut s0_recv) = quic_conn.open_bi().await.expect("open Stream 0");
    omnilink_daemon::quic::write_control_packet(&mut s0_send, &ControlPacket::Ping { timestamp: 100 })
        .await
        .unwrap();
    let _ = read_control_packet(&mut s0_recv).await.unwrap();

    // Open Stream 1
    let (mut s1_send, _s1_recv) = quic_conn.open_bi().await.expect("open Stream 1");
    let init_clip = omnilink_protocol::packet::ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    omnilink_daemon::quic::write_clipboard_packet(&mut s1_send, &init_clip).await.unwrap();

    // Open Stream 2
    let (mut s2_send, mut s2_recv) = quic_conn.open_bi().await.expect("open Stream 2");
    let init_notif = NotificationStreamPacket::Posted(omnilink_protocol::packet::NotificationPacket {
        id: "init".to_string(),
        package_name: "pkg".to_string(),
        app_name: "App".to_string(),
        title: "T".to_string(),
        text: "B".to_string(),
        icon_png: None,
        timestamp: 10,
        can_reply: false,
        actions: vec![],
    });
    omnilink_daemon::quic::write_notification_packet(&mut s2_send, &init_notif).await.unwrap();

    let mut client2 = IpcClient::connect(&path2).await.expect("connect client 2");

    // Send reply via IPC
    let resp_reply = client2.request(&reply_req).await.expect("reply connected");
    assert_eq!(resp_reply.result, Some(serde_json::json!({ "success": true })));

    let received_packet = read_notification_packet(&mut s2_recv)
        .await
        .expect("read reply on Stream 2");
    assert_eq!(
        received_packet,
        NotificationStreamPacket::Reply(NotificationReplyPacket {
            notification_id: "0|com.whatsapp|1001".to_string(),
            action_key: "inline-reply".to_string(),
            reply_text: "Sounds good!".to_string(),
        })
    );

    // Send dismiss via IPC
    let resp_dismiss = client2.request(&dismiss_req).await.expect("dismiss connected");
    assert_eq!(resp_dismiss.result, Some(serde_json::json!({ "success": true })));

    let received_packet = read_notification_packet(&mut s2_recv)
        .await
        .expect("read dismiss on Stream 2");
    assert_eq!(
        received_packet,
        NotificationStreamPacket::Dismiss(NotificationDismissPacket {
            id: "0|com.whatsapp|1001".to_string(),
        })
    );

    let _ = s2_send.finish();
    ipc_handle2.stop().await;
}

#[tokio::test]
async fn test_ipc_error_handling() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let handle = IpcServer::bind(Some(&path), state).await.expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // 1. Invalid JSON (-32700)
    client.send_raw("not valid json\n").await.expect("send raw");
    let raw_resp = client.next_line().await.expect("read resp").expect("some line");
    let val: serde_json::Value = serde_json::from_str(&raw_resp).expect("parse json");
    assert_eq!(val["error"]["code"], -32700);

    // 2. Method not found (-32601)
    let unknown_req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(100)),
        method: "nonexistent.method".to_string(),
        params: None,
    };
    let resp = client.request(&unknown_req).await.expect("request unknown");
    assert_eq!(resp.error.unwrap().code, -32601);

    // 3. Invalid params: string instead of object for notification.reply (-32602)
    let bad_params_req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(101)),
        method: "notification.reply".to_string(),
        params: Some(serde_json::json!("string_is_invalid")),
    };
    let resp = client.request(&bad_params_req).await.expect("request bad params");
    assert_eq!(resp.error.unwrap().code, -32602);

    // 4. Invalid params: missing required fields for notification.reply (-32602)
    let missing_fields_req = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(102)),
        method: "notification.reply".to_string(),
        params: Some(serde_json::json!({ "notification_id": "notif-1" })), // missing reply_text
    };
    let resp = client.request(&missing_fields_req).await.expect("request missing fields");
    assert_eq!(resp.error.unwrap().code, -32602);

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_event_broadcast_to_multiple_clients() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let handle = IpcServer::bind(Some(&path), state.clone()).await.expect("bind server");

    let mut client1 = IpcClient::connect(&path).await.expect("connect client 1");
    let mut client2 = IpcClient::connect(&path).await.expect("connect client 2");

    // Allow background subscriptions to settle
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Broadcast device.status_changed
    let status = DeviceStatus {
        battery_pct: 81,
        is_charging: true,
        wifi_ssid: Some("Office-WiFi".to_string()),
        cellular_signal: 3,
        hotspot_active: true,
    };
    state.broadcast_device_status(&status);

    // 2. Broadcast clipboard.updated
    state.broadcast_clipboard_updated("text/plain", "https://omarchy.org");

    // Verify client 1 receives both events
    let c1_line1 = client1.next_line().await.unwrap().unwrap();
    let c1_event1: IpcEvent = serde_json::from_str(&c1_line1).unwrap();
    assert_eq!(c1_event1.event, "device.status_changed");
    assert_eq!(c1_event1.data["battery_pct"], 81);
    assert_eq!(c1_event1.data["is_charging"], true);

    let c1_line2 = client1.next_line().await.unwrap().unwrap();
    let c1_event2: IpcEvent = serde_json::from_str(&c1_line2).unwrap();
    assert_eq!(c1_event2.event, "clipboard.updated");
    assert_eq!(c1_event2.data["mime_type"], "text/plain");
    assert_eq!(c1_event2.data["preview"], "https://omarchy.org");

    // Verify client 2 receives both events
    let c2_line1 = client2.next_line().await.unwrap().unwrap();
    let c2_event1: IpcEvent = serde_json::from_str(&c2_line1).unwrap();
    assert_eq!(c2_event1.event, "device.status_changed");
    assert_eq!(c2_event1.data["battery_pct"], 81);

    let c2_line2 = client2.next_line().await.unwrap().unwrap();
    let c2_event2: IpcEvent = serde_json::from_str(&c2_line2).unwrap();
    assert_eq!(c2_event2.event, "clipboard.updated");
    assert_eq!(c2_event2.data["preview"], "https://omarchy.org");

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_client_disconnect_resilience() {
    let path = temp_socket_path();
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let handle = IpcServer::bind(Some(&path), state.clone()).await.expect("bind server");

    // Client 1 connects, sends a request, and disconnects abruptly
    {
        let mut client1 = IpcClient::connect(&path).await.expect("connect client 1");
        let req = JsonRpcRequest {
            jsonrpc: Some("2.0".to_string()),
            id: Some(serde_json::json!(1)),
            method: "ping".to_string(),
            params: None,
        };
        let resp = client1.request(&req).await.expect("client1 ping");
        assert_eq!(resp.result, Some(serde_json::json!("pong")));
        // client1 is dropped here (abrupt TCP/UNIX socket close)
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Daemon should still be running and accept client 2 without issues
    let mut client2 = IpcClient::connect(&path).await.expect("connect client 2");
    let req2 = JsonRpcRequest {
        jsonrpc: Some("2.0".to_string()),
        id: Some(serde_json::json!(2)),
        method: "ping".to_string(),
        params: None,
    };
    let resp2 = client2.request(&req2).await.expect("client2 ping");
    assert_eq!(resp2.result, Some(serde_json::json!("pong")));

    // Emitting broadcast event after client disconnect does not crash
    state.broadcast_clipboard_updated("text/plain", "test preview");

    let line = client2.next_line().await.unwrap().unwrap();
    let event: IpcEvent = serde_json::from_str(&line).unwrap();
    assert_eq!(event.event, "clipboard.updated");

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_sensitive_clipboard_masking_and_preview_truncation() {
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let path = temp_socket_path();
    let handle = IpcServer::bind(Some(&path), state.clone()).await.expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // Allow background subscriptions to settle
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Sensitive clipboard packet
    let sensitive_packet = omnilink_protocol::packet::ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".to_string(),
        payload: b"my-super-secret-password".to_vec(),
        is_sensitive: true,
    };
    state.notify_inbound_clipboard(sensitive_packet).await;

    let line1 = client.next_line().await.unwrap().unwrap();
    let event1: IpcEvent = serde_json::from_str(&line1).unwrap();
    assert_eq!(event1.event, "clipboard.updated");
    assert_eq!(event1.data["preview"], "<sensitive content redacted>");

    // 2. Long text clipboard packet
    let long_text = "A".repeat(200);
    let long_packet = omnilink_protocol::packet::ClipboardPacket {
        sequence_id: 2,
        mime_type: "text/plain".to_string(),
        payload: long_text.into_bytes(),
        is_sensitive: false,
    };
    state.notify_inbound_clipboard(long_packet).await;

    let line2 = client.next_line().await.unwrap().unwrap();
    let event2: IpcEvent = serde_json::from_str(&line2).unwrap();
    assert_eq!(event2.event, "clipboard.updated");
    let preview = event2.data["preview"].as_str().unwrap();
    assert!(preview.ends_with("..."));
    assert_eq!(preview.len(), 123); // 120 + 3

    handle.stop().await;
}

#[tokio::test]
async fn test_ipc_oversized_line_rejected() {
    let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
    let state = Arc::new(DaemonState::new(signing_key, None));

    let path = temp_socket_path();
    let handle = IpcServer::bind(Some(&path), state.clone()).await.expect("bind server");
    let mut client = IpcClient::connect(&path).await.expect("connect client");

    // Oversized line > 64 KB
    let huge_line = "{\"id\":1,\"method\":\"ping\",\"params\":{\"data\":\"".to_string()
        + &"x".repeat(70 * 1024)
        + "\"}}";
    client.send_raw(&huge_line).await.expect("send huge line");
    let resp = client.next_line().await.unwrap().unwrap();
    let parsed: omnilink_daemon::JsonRpcResponse = serde_json::from_str(&resp).unwrap();
    assert_eq!(parsed.error.unwrap().code, -32600); // Invalid request

    handle.stop().await;
}
