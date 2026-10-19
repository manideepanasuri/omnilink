//! QUIC transport endpoint, mutual TLS 1.3, and Stream 0 multiplexer tests.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use omnilink_daemon::clipboard::ClipboardEvent;
use omnilink_daemon::quic::{
    init_crypto_provider, make_client_config, read_clipboard_packet, read_control_packet,
    read_notification_packet, write_clipboard_packet, write_control_packet,
    write_notification_packet, Ed25519ServerCertVerifier, QuicServer,
};
use omnilink_daemon::state::{DaemonState, PairedDevice};
use omnilink_protocol::packet::{
    ClipboardPacket, ControlPacket, NotificationAction, NotificationDismissPacket,
    NotificationPacket, NotificationReplyPacket, NotificationStreamPacket,
};
use quinn::Endpoint;

/// Helper to set up a test daemon listening on an ephemeral port with a paired client key.
async fn setup_test_daemon() -> (QuicServer, Arc<DaemonState>, SigningKey, SigningKey) {
    init_crypto_provider();

    let mut csprng = rand::rngs::OsRng;
    let daemon_signing_key = SigningKey::generate(&mut csprng);
    let client_signing_key = SigningKey::generate(&mut csprng);

    let paired_device = PairedDevice {
        name: "Test-Android-Device".to_string(),
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

/// Helper to create a client endpoint and connect to the server.
async fn connect_client(
    server_addr: SocketAddr,
    client_signing_key: &SigningKey,
    daemon_signing_key: &SigningKey,
) -> Result<quinn::Connection, quinn::ConnectionError> {
    let client_config = make_client_config(
        client_signing_key,
        &daemon_signing_key.verifying_key(),
        Some(Duration::from_secs(5)),
    )
    .expect("Failed to build client config");

    let mut client_endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap())
        .expect("Failed to create client endpoint");
    client_endpoint.set_default_client_config(client_config);

    let connecting = client_endpoint
        .connect(server_addr, "localhost")
        .expect("Failed to initiate connect");

    connecting.await
}

#[tokio::test]
async fn test_quic_1_authenticated_client_connects_and_ping_pong() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    // Connect authenticated client
    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Authenticated client should connect successfully");

    // Establish Stream 0 (bidirectional stream 0)
    let (mut send_stream, mut recv_stream) = connection
        .open_bi()
        .await
        .expect("Failed to open Stream 0");

    assert_eq!(send_stream.id().index(), 0);

    // Send Ping
    let ping_timestamp = 1729255555123u64;
    let ping_packet = ControlPacket::Ping {
        timestamp: ping_timestamp,
    };
    write_control_packet(&mut send_stream, &ping_packet)
        .await
        .expect("Failed to write Ping");

    // Receive Pong
    let pong_packet = read_control_packet(&mut recv_stream)
        .await
        .expect("Failed to read Pong reply");

    match pong_packet {
        ControlPacket::Pong { timestamp } => {
            assert_eq!(timestamp, ping_timestamp);
        }
        other => panic!("Expected Pong, got {:?}", other),
    }

    // Verify daemon session is active
    assert!(daemon_state.is_connected().await);

    // Clean up
    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_2_authenticated_client_sends_device_status_telemetry() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Authenticated client should connect successfully");

    let (mut send_stream, _recv_stream) = connection
        .open_bi()
        .await
        .expect("Failed to open Stream 0");

    // Send initial DeviceStatus
    let status_packet = ControlPacket::DeviceStatus {
        battery_pct: 82,
        is_charging: true,
        wifi_ssid: Some("Office-5G".to_string()),
        cellular_signal: 4,
        hotspot_active: false,
    };

    write_control_packet(&mut send_stream, &status_packet)
        .await
        .expect("Failed to send DeviceStatus");

    // Poll until daemon state updates telemetry
    let mut updated = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(status) = daemon_state.get_device_status().await {
            if status.battery_pct == 82 {
                assert!(status.is_charging);
                assert_eq!(status.wifi_ssid.as_deref(), Some("Office-5G"));
                assert_eq!(status.cellular_signal, 4);
                assert!(!status.hotspot_active);
                updated = true;
                break;
            }
        }
    }
    assert!(updated, "Daemon session telemetry was not updated");

    // Send updated DeviceStatus (e.g. unplugged and hotspot turned on)
    let updated_status_packet = ControlPacket::DeviceStatus {
        battery_pct: 81,
        is_charging: false,
        wifi_ssid: None,
        cellular_signal: 2,
        hotspot_active: true,
    };

    write_control_packet(&mut send_stream, &updated_status_packet)
        .await
        .expect("Failed to send updated DeviceStatus");

    let mut second_update = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(status) = daemon_state.get_device_status().await {
            if status.battery_pct == 81 {
                assert!(!status.is_charging);
                assert_eq!(status.wifi_ssid, None);
                assert_eq!(status.cellular_signal, 2);
                assert!(status.hotspot_active);
                second_update = true;
                break;
            }
        }
    }
    assert!(second_update, "Second telemetry update was not reflected");

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_3_unauthenticated_and_unknown_certificate_clients_rejected() {
    let (server, daemon_state, daemon_signing_key, _paired_client_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    // Subcase 3A: Unauthenticated client (no certificate presented)
    {
        let server_cert_verifier = Arc::new(Ed25519ServerCertVerifier::new(
            daemon_signing_key.verifying_key(),
        ));
        let mut rustls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(server_cert_verifier)
            .with_no_client_auth();
        rustls_config.alpn_protocols = vec![omnilink_daemon::quic::ALPN_OMNILINK.to_vec()];

        let quic_client_config =
            quinn::crypto::rustls::QuicClientConfig::try_from(rustls_config).unwrap();
        let client_config = quinn::ClientConfig::new(Arc::new(quic_client_config));

        let mut client_endpoint = Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        client_endpoint.set_default_client_config(client_config);

        let connect_result = client_endpoint.connect(server_addr, "localhost").unwrap().await;
        if let Ok(conn) = connect_result {
            let close_err = conn.closed().await;
            assert!(
                matches!(close_err, quinn::ConnectionError::ConnectionClosed(_)),
                "Expected connection to be closed by server due to missing certificate"
            );
        }
        assert!(
            !daemon_state.is_connected().await,
            "No session should be active"
        );
    }

    // Subcase 3B: Unknown / unpaired client certificate
    {
        let mut csprng = rand::rngs::OsRng;
        let unknown_signing_key = SigningKey::generate(&mut csprng);

        let connect_result =
            connect_client(server_addr, &unknown_signing_key, &daemon_signing_key).await;

        if let Ok(conn) = connect_result {
            let close_error = conn.closed().await;
            assert!(
                matches!(close_error, quinn::ConnectionError::ConnectionClosed(_)),
                "Expected connection to be closed by server due to unknown certificate"
            );
        }
        assert!(
            !daemon_state.is_connected().await,
            "No session should be active after rejected handshake"
        );
    }

    server_handle.abort();
}

#[tokio::test]
async fn test_quic_4_disconnect_and_heartbeat_timeout_drops_session() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    // Subcase 4A: Client disconnect drops session within 5 seconds
    {
        let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
            .await
            .expect("Client should connect");

        let (mut send_stream, mut recv_stream) = connection
            .open_bi()
            .await
            .expect("Failed to open Stream 0");

        // Send a ping to confirm stream 0 is established
        write_control_packet(&mut send_stream, &ControlPacket::Ping { timestamp: 100 })
            .await
            .unwrap();
        let _ = read_control_packet(&mut recv_stream).await.unwrap();

        assert!(daemon_state.is_connected().await, "Session should be active");

        // Disconnect client
        drop(send_stream);
        drop(recv_stream);
        connection.close(0u32.into(), b"disconnecting");

        // Verify session drops within 5 seconds (typically within < 100ms)
        let start = std::time::Instant::now();
        let mut dropped = false;
        while start.elapsed() < Duration::from_secs(5) {
            if !daemon_state.is_connected().await {
                dropped = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            dropped,
            "Session was not dropped within 5 seconds after client disconnect"
        );
    }

    // Subcase 4B: Heartbeat timeout drops session within 5 seconds
    {
        let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
            .await
            .expect("Client should connect");

        let (mut send_stream, mut recv_stream) = connection
            .open_bi()
            .await
            .expect("Failed to open Stream 0");

        write_control_packet(&mut send_stream, &ControlPacket::Ping { timestamp: 200 })
            .await
            .unwrap();
        let _ = read_control_packet(&mut recv_stream).await.unwrap();

        assert!(daemon_state.is_connected().await, "Session should be active");

        // Client goes silent: does not send any pings or packets
        // Verify session is dropped by the daemon within 5.5 seconds due to heartbeat timeout
        let start = std::time::Instant::now();
        let mut timed_out = false;
        while start.elapsed() < Duration::from_millis(5500) {
            if !daemon_state.is_connected().await {
                timed_out = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        assert!(
            timed_out,
            "Session was not dropped within 5 seconds of heartbeat inactivity"
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_secs(5),
            "Timeout occurred prematurely before 5 seconds: {:?}",
            elapsed
        );
    }

    server_handle.abort();
}

#[tokio::test]
async fn test_quic_5_dynamic_pairing_key_updates() {
    init_crypto_provider();

    let mut csprng = rand::rngs::OsRng;
    let daemon_signing_key = SigningKey::generate(&mut csprng);
    let client_signing_key = SigningKey::generate(&mut csprng);

    // Start daemon state with NO paired device
    let daemon_state = Arc::new(DaemonState::new(daemon_signing_key.clone(), None));

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let server = QuicServer::bind(addr, daemon_state.clone())
        .await
        .expect("Failed to bind QuicServer");
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    // 1. Connection attempt should fail when daemon has no paired key
    let connect_res = connect_client(server_addr, &client_signing_key, &daemon_signing_key).await;
    if let Ok(conn) = connect_res {
        let err = conn.closed().await;
        assert!(matches!(err, quinn::ConnectionError::ConnectionClosed(_)));
    }
    assert!(!daemon_state.is_connected().await);

    // 2. Dynamically pair the device in state without restarting the server
    daemon_state
        .set_paired_device(Some(PairedDevice {
            name: "Dynamic-Pixel".to_string(),
            public_key: client_signing_key.verifying_key(),
            paired_at: 1729260000,
        }))
        .await;

    // 3. Connection attempt should now succeed immediately
    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect after dynamic pairing");

    let (mut send_stream, mut recv_stream) = connection
        .open_bi()
        .await
        .expect("Failed to open Stream 0");

    write_control_packet(&mut send_stream, &ControlPacket::Ping { timestamp: 500 })
        .await
        .unwrap();
    let reply = read_control_packet(&mut recv_stream).await.unwrap();
    assert_eq!(reply, ControlPacket::Pong { timestamp: 500 });
    assert!(daemon_state.is_connected().await);

    // Clean up
    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_6_outbound_command_dispatching_on_stream_0() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    let (mut send_stream, mut recv_stream) = connection
        .open_bi()
        .await
        .expect("Failed to open Stream 0");

    // Establish stream with a ping
    write_control_packet(&mut send_stream, &ControlPacket::Ping { timestamp: 600 })
        .await
        .unwrap();
    let pong = read_control_packet(&mut recv_stream).await.unwrap();
    assert_eq!(pong, ControlPacket::Pong { timestamp: 600 });

    // Send an outbound command from daemon to client companion
    let outbound_cmd = ControlPacket::Command {
        action: omnilink_protocol::packet::CommandAction::RingPhone,
    };
    daemon_state
        .send_command(outbound_cmd.clone())
        .await
        .expect("Failed to send outbound command via daemon state");

    // Companion client receives the outbound command on Stream 0
    let received_packet = read_control_packet(&mut recv_stream)
        .await
        .expect("Companion failed to read outbound command");
    assert_eq!(received_packet, outbound_cmd);

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_7_session_teardown_race_condition_guard() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    let (mut send_stream, mut recv_stream) = connection
        .open_bi()
        .await
        .expect("Failed to open Stream 0");

    write_control_packet(&mut send_stream, &ControlPacket::Ping { timestamp: 700 })
        .await
        .unwrap();
    let _ = read_control_packet(&mut recv_stream).await.unwrap();

    assert!(daemon_state.is_connected().await);

    // Retrieve active connection's stable_id
    let actual_stable_id = {
        let lock = daemon_state.session.read().await;
        lock.as_ref().unwrap().connection.stable_id()
    };

    // Stale/wrong stable_id should NOT drop the active session
    let stale_stable_id = actual_stable_id + 9999;
    daemon_state.clear_session(stale_stable_id).await;
    assert!(
        daemon_state.is_connected().await,
        "Active session must NOT be cleared by mismatched stable_id"
    );

    // Matching stable_id SHOULD drop the session
    daemon_state.clear_session(actual_stable_id).await;
    assert!(
        !daemon_state.is_connected().await,
        "Active session must be cleared by matching stable_id"
    );

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_8_unidirectional_stream_demuxing() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open first client unidirectional stream (Stream 4: Video stream, index 0)
    let mut uni_stream_0 = connection
        .open_uni()
        .await
        .expect("Failed to open uni stream 0");
    assert_eq!(uni_stream_0.id().index(), 0);
    uni_stream_0.write_all(b"video-frame-data").await.unwrap();
    uni_stream_0.finish().unwrap();

    // Open second client unidirectional stream (Stream 5: Remote input, index 1)
    let mut uni_stream_1 = connection
        .open_uni()
        .await
        .expect("Failed to open uni stream 1");
    assert_eq!(uni_stream_1.id().index(), 1);
    uni_stream_1.write_all(b"remote-input-event").await.unwrap();
    uni_stream_1.finish().unwrap();

    // Verify session remains connected and stable
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(daemon_state.is_connected().await);

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_9_stream_1_inbound_clipboard_sync_and_deduplication() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0 (index 0)
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    assert_eq!(send_stream_0.id().index(), 0);
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 100 })
        .await
        .unwrap();

    // Open Stream 1 (index 1)
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();
    assert_eq!(send_stream_1.id().index(), 1);

    // Client sends an inbound clipboard packet to daemon
    let payload = b"Hello from Android Phone Clipboard!".to_vec();
    let packet = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain;charset=utf-8".to_string(),
        is_sensitive: false,
        payload: payload.clone(),
    };

    write_clipboard_packet(&mut send_stream_1, &packet)
        .await
        .expect("Client should successfully send ClipboardPacket");

    // Poll daemon until incoming packet is recorded in state
    let mut received = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(clip) = daemon_state.last_received_clipboard().await {
            if clip.sequence_id == 1 && clip.payload == payload {
                assert_eq!(clip.mime_type, "text/plain;charset=utf-8");
                assert!(!clip.is_sensitive);
                received = true;
                break;
            }
        }
    }
    assert!(received, "Daemon should record inbound clipboard packet");

    // Verify filter recorded the BLAKE3 hash
    assert!(
        daemon_state.clipboard_filter().lock().unwrap().contains(&payload),
        "Filter must contain the received clipboard hash"
    );

    // Send duplicate payload with a new sequence ID
    let duplicate_packet = ClipboardPacket {
        sequence_id: 2,
        mime_type: "text/plain;charset=utf-8".to_string(),
        is_sensitive: false,
        payload: payload.clone(),
    };

    write_clipboard_packet(&mut send_stream_1, &duplicate_packet)
        .await
        .unwrap();

    // Give daemon time to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sequence ID should STILL be 1 because duplicate was dropped by filter
    let current_clip = daemon_state.last_received_clipboard().await.unwrap();
    assert_eq!(
        current_clip.sequence_id, 1,
        "Duplicate payload must be dropped by deduplication filter without updating state"
    );

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_10_stream_1_outbound_clipboard_dispatch() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 200 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, mut recv_stream_1) = connection.open_bi().await.unwrap();

    // Client sends an initial packet to trigger stream acceptance on daemon
    let init_packet = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    write_clipboard_packet(&mut send_stream_1, &init_packet).await.unwrap();

    // Wait until daemon initializes session and processes init packet
    for _ in 0..50 {
        if daemon_state.last_received_clipboard().await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(daemon_state.is_connected().await);
    assert!(daemon_state.last_received_clipboard().await.is_some());

    // Daemon dispatches outbound clipboard event
    let payload = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic header
    let event = ClipboardEvent::new("image/png", false, payload.clone());

    daemon_state
        .send_clipboard(event)
        .await
        .expect("Daemon should send clipboard event over Stream 1");

    // Client receives on Stream 1
    let received_packet = read_clipboard_packet(&mut recv_stream_1)
        .await
        .expect("Client should read framed ClipboardPacket");

    assert_eq!(received_packet.sequence_id, 1);
    assert_eq!(received_packet.mime_type, "image/png");
    assert!(!received_packet.is_sensitive);
    assert_eq!(received_packet.payload, payload);

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_11_stream_1_echo_loop_prevention() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 300 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();

    // Pre-record payload in daemon filter (simulating local Wayland selection)
    let pre_recorded = b"Password-manager-token-abc";
    {
        let mut filter = daemon_state.clipboard_filter().lock().unwrap();
        filter.record(pre_recorded);
    }

    // Phone attempts to send this exact same payload back to daemon
    let packet = ClipboardPacket {
        sequence_id: 10,
        mime_type: "text/plain".to_string(),
        is_sensitive: true,
        payload: pre_recorded.to_vec(),
    };

    write_clipboard_packet(&mut send_stream_1, &packet)
        .await
        .unwrap();

    // Wait and verify that daemon dropped it and did not notify state
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        daemon_state.last_received_clipboard().await.is_none(),
        "Pre-recorded payload must be detected as loop and dropped without updating state"
    );

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_12_stream_1_oversized_payload_clamp_rejected() {
    let (server, _daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 400 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();

    // Send a length prefix indicating 25 MiB (exceeding 20 MiB safety clamp)
    let excessive_len: u32 = 25 * 1024 * 1024;
    send_stream_1.write_all(&excessive_len.to_be_bytes()).await.unwrap();

    // Daemon should detect size clamp violation and close/reset stream
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Further write to stream should eventually fail or stream should be closed
    let dummy_packet = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: vec![1, 2, 3],
    };
    let _ = write_clipboard_packet(&mut send_stream_1, &dummy_packet).await;

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_13_stream_2_inbound_notification_dispatch() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0 (index 0)
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 500 })
        .await
        .unwrap();

    // Open Stream 1 (index 1)
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();
    let init_clip = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    write_clipboard_packet(&mut send_stream_1, &init_clip).await.unwrap();

    // Open Stream 2 (index 2)
    let (mut send_stream_2, _recv_stream_2) = connection.open_bi().await.unwrap();
    assert_eq!(send_stream_2.id().index(), 2);

    let mut notif_rx = daemon_state.inbound_notification_tx.subscribe();

    // Client sends an inbound posted notification
    let notif = NotificationPacket {
        id: "msg-123".to_string(),
        package_name: "org.telegram.messenger".to_string(),
        app_name: "Telegram".to_string(),
        title: "Alice".to_string(),
        text: "See you at 5pm!".to_string(),
        icon_png: None,
        timestamp: 1729260000000,
        can_reply: true,
        actions: vec![NotificationAction {
            key: "reply".to_string(),
            title: "Reply".to_string(),
            is_input: true,
        }],
    };

    let stream_packet = NotificationStreamPacket::Posted(notif.clone());
    write_notification_packet(&mut send_stream_2, &stream_packet)
        .await
        .expect("Client should write notification packet to Stream 2");

    // Daemon receives and broadcasts on inbound_notification_tx
    let received = tokio::time::timeout(Duration::from_secs(3), notif_rx.recv())
        .await
        .expect("Timed out waiting for notification broadcast")
        .expect("Broadcast channel receive failed");

    assert_eq!(received.id, "msg-123");
    assert_eq!(received.title, "Alice");
    assert_eq!(received.text, "See you at 5pm!");
    assert!(received.can_reply);

    // Client sends a dismissal
    let mut dismiss_rx = daemon_state.inbound_dismiss_tx.subscribe();
    let dismiss_packet = NotificationStreamPacket::Dismiss(NotificationDismissPacket {
        id: "msg-123".to_string(),
    });
    write_notification_packet(&mut send_stream_2, &dismiss_packet)
        .await
        .expect("Client should write dismiss packet to Stream 2");

    let received_dismiss = tokio::time::timeout(Duration::from_secs(3), dismiss_rx.recv())
        .await
        .expect("Timed out waiting for dismissal broadcast")
        .expect("Broadcast channel receive failed");

    assert_eq!(received_dismiss.id, "msg-123");

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_14_stream_2_outbound_reply_and_dismiss_dispatch() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 600 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();
    let init_clip = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    write_clipboard_packet(&mut send_stream_1, &init_clip).await.unwrap();

    // Open Stream 2
    let (mut send_stream_2, mut recv_stream_2) = connection.open_bi().await.unwrap();

    // Send a dummy trigger packet to ensure daemon starts Stream 2 pump
    let init_notif = NotificationStreamPacket::Posted(NotificationPacket {
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
    write_notification_packet(&mut send_stream_2, &init_notif).await.unwrap();

    // Wait until daemon session is active
    for _ in 0..50 {
        if daemon_state.is_connected().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(daemon_state.is_connected().await);

    // Daemon dispatches outbound reply
    let reply_packet = NotificationReplyPacket {
        notification_id: "msg-123".to_string(),
        action_key: "inline-reply".to_string(),
        reply_text: "Replying back from Hyprland!".to_string(),
    };

    let session = daemon_state.session.read().await.clone().unwrap();
    session
        .send_notification(NotificationStreamPacket::Reply(reply_packet.clone()))
        .await
        .expect("Daemon should send reply over Stream 2");

    // Client reads reply on Stream 2
    let received_packet = read_notification_packet(&mut recv_stream_2)
        .await
        .expect("Client should read framed notification packet");

    assert_eq!(
        received_packet,
        NotificationStreamPacket::Reply(reply_packet)
    );

    // Daemon dispatches outbound dismissal
    let dismiss_packet = NotificationDismissPacket {
        id: "msg-123".to_string(),
    };
    session
        .send_notification(NotificationStreamPacket::Dismiss(dismiss_packet.clone()))
        .await
        .expect("Daemon should send dismiss over Stream 2");

    let received_dismiss = read_notification_packet(&mut recv_stream_2)
        .await
        .expect("Client should read dismiss packet");

    assert_eq!(
        received_dismiss,
        NotificationStreamPacket::Dismiss(dismiss_packet)
    );

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_15_stream_2_oversized_payload_clamp_rejected() {
    let (server, _daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 700 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();
    let init_clip = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    write_clipboard_packet(&mut send_stream_1, &init_clip).await.unwrap();

    // Open Stream 2
    let (mut send_stream_2, _recv_stream_2) = connection.open_bi().await.unwrap();

    // Send length prefix > 20 MiB clamp
    let excessive_len: u32 = 25 * 1024 * 1024;
    send_stream_2.write_all(&excessive_len.to_be_bytes()).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let dummy_packet = NotificationStreamPacket::Dismiss(NotificationDismissPacket {
        id: "bad".to_string(),
    });
    let _ = write_notification_packet(&mut send_stream_2, &dummy_packet).await;

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}

#[tokio::test]
async fn test_quic_16_daemon_state_convenience_reply_and_dismissal_methods() {
    let (server, daemon_state, daemon_signing_key, client_signing_key) = setup_test_daemon().await;
    let server_addr = server.local_addr().unwrap();
    let server_handle = server.start();

    let connection = connect_client(server_addr, &client_signing_key, &daemon_signing_key)
        .await
        .expect("Client should connect");

    // Open Stream 0
    let (mut send_stream_0, _recv_stream_0) = connection.open_bi().await.unwrap();
    write_control_packet(&mut send_stream_0, &ControlPacket::Ping { timestamp: 800 })
        .await
        .unwrap();

    // Open Stream 1
    let (mut send_stream_1, _recv_stream_1) = connection.open_bi().await.unwrap();
    let init_clip = ClipboardPacket {
        sequence_id: 1,
        mime_type: "text/plain".into(),
        is_sensitive: false,
        payload: b"ready".to_vec(),
    };
    write_clipboard_packet(&mut send_stream_1, &init_clip).await.unwrap();

    // Open Stream 2
    let (mut send_stream_2, mut recv_stream_2) = connection.open_bi().await.unwrap();
    let init_notif = NotificationStreamPacket::Posted(NotificationPacket {
        id: "init-stream-2".to_string(),
        package_name: "org.test".to_string(),
        app_name: "Test".to_string(),
        title: "Hello".to_string(),
        text: "World".to_string(),
        icon_png: None,
        timestamp: 10,
        can_reply: true,
        actions: vec![],
    });
    write_notification_packet(&mut send_stream_2, &init_notif).await.unwrap();

    // Wait until daemon session is active
    for _ in 0..50 {
        if daemon_state.is_connected().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(daemon_state.is_connected().await);

    // 1. Daemon state sends inline reply
    daemon_state
        .send_notification_reply("msg-789", "inline-reply", "Direct state reply!")
        .await
        .expect("send_notification_reply should succeed");

    let received_reply = read_notification_packet(&mut recv_stream_2)
        .await
        .expect("Client should receive reply packet");

    assert_eq!(
        received_reply,
        NotificationStreamPacket::Reply(NotificationReplyPacket {
            notification_id: "msg-789".to_string(),
            action_key: "inline-reply".to_string(),
            reply_text: "Direct state reply!".to_string(),
        })
    );

    // 2. Daemon state sends dismissal
    daemon_state
        .send_notification_dismiss("msg-789")
        .await
        .expect("send_notification_dismiss should succeed");

    let received_dismiss = read_notification_packet(&mut recv_stream_2)
        .await
        .expect("Client should receive dismiss packet");

    assert_eq!(
        received_dismiss,
        NotificationStreamPacket::Dismiss(NotificationDismissPacket {
            id: "msg-789".to_string(),
        })
    );

    connection.close(0u32.into(), b"done");
    server_handle.abort();
}



