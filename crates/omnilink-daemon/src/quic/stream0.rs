//! Stream 0 control loop, telemetry updates, Ping/Pong heartbeats, and timeout detection.

use std::sync::Arc;
use std::time::Duration;

use omnilink_protocol::packet::ControlPacket;
use omnilink_protocol::types::DeviceStatus;
use quinn::{RecvStream, SendStream};

use crate::quic::QuicError;
use crate::state::DaemonState;

/// Maximum allowable payload size for Stream 0 control packets (64 KB).
pub const MAX_CONTROL_PACKET_LEN: usize = 64 * 1024;

/// Connection heartbeat timeout duration (5 seconds).
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);

/// Reads a single framed `ControlPacket` from a QUIC receive stream.
///
/// Frame format: `[len: 4 bytes BE u32][payload: len bytes]`.
/// Decodes from MessagePack with fallback to JSON.
pub async fn read_control_packet(recv_stream: &mut RecvStream) -> Result<ControlPacket, QuicError> {
    let mut len_buf = [0u8; 4];
    match recv_stream.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => {
            return Err(QuicError::ConnectionClosed(
                "Stream 0 closed by peer".to_string(),
            ));
        }
        Err(e) => return Err(QuicError::ReadExact(e)),
    }

    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len > MAX_CONTROL_PACKET_LEN {
        return Err(QuicError::Protocol(
            omnilink_protocol::error::ProtocolError::InvalidHeader(format!(
                "Control packet exceeds maximum size limit ({} > {})",
                payload_len, MAX_CONTROL_PACKET_LEN
            )),
        ));
    }

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        match recv_stream.read_exact(&mut payload).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => {
                return Err(QuicError::ConnectionClosed(
                    "Truncated control packet received on Stream 0".to_string(),
                ));
            }
            Err(e) => return Err(QuicError::ReadExact(e)),
        }
    }

    // Attempt MessagePack decoding first
    match ControlPacket::from_msgpack(&payload) {
        Ok(packet) => Ok(packet),
        Err(msgpack_err) => {
            // Attempt JSON fallback
            if let Ok(json_str) = std::str::from_utf8(&payload) {
                if let Ok(packet) = ControlPacket::from_json(json_str) {
                    return Ok(packet);
                }
            }
            Err(QuicError::Protocol(msgpack_err))
        }
    }
}

/// Encodes and writes a framed `ControlPacket` to a QUIC send stream.
///
/// Frame format: `[len: 4 bytes BE u32][payload: len bytes]`.
/// Combines the 4-byte length prefix and payload into a single write buffer to prevent chunk fragmentation.
pub async fn write_control_packet(
    send_stream: &mut SendStream,
    packet: &ControlPacket,
) -> Result<(), QuicError> {
    let payload = packet.to_json()?.into_bytes();
    let len = payload.len() as u32;

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);

    send_stream.write_all(&buf).await?;

    Ok(())
}

/// Handles the Stream 0 control loop for an authenticated connection.
///
/// Responsibilities:
/// - Replies to `Ping { timestamp }` with `Pong { timestamp }`.
/// - Updates device telemetry on `DeviceStatus { ... }`.
/// - Dispatches outgoing commands from `command_rx` to `send_stream`.
/// - Detects heartbeat timeout or connection disconnect within 5 seconds and drops the session.
pub async fn run_stream_0(
    mut send_stream: SendStream,
    mut recv_stream: RecvStream,
    state: Arc<DaemonState>,
    mut command_rx: tokio::sync::mpsc::Receiver<ControlPacket>,
    stable_id: usize,
) -> Result<(), QuicError> {
    tracing::info!(stable_id, "Stream 0 control loop started");

    let heartbeat_sleep = tokio::time::sleep(HEARTBEAT_TIMEOUT);
    tokio::pin!(heartbeat_sleep);

    let mut command_rx_open = true;

    loop {
        tokio::select! {
            () = &mut heartbeat_sleep => {
                tracing::warn!(stable_id, "Stream 0 heartbeat timeout (> 5 seconds), clearing session");
                state.clear_session(stable_id).await;
                return Err(QuicError::HeartbeatTimeout);
            }
            cmd_opt = command_rx.recv(), if command_rx_open => {
                match cmd_opt {
                    Some(packet) => {
                        tracing::debug!(stable_id, ?packet, "Stream 0 dispatching outbound command");
                        if let Err(e) = write_control_packet(&mut send_stream, &packet).await {
                            tracing::warn!(stable_id, error = %e, "Failed to send outbound command on Stream 0");
                            state.clear_session(stable_id).await;
                            return Err(e);
                        }
                    }
                    None => {
                        tracing::debug!(stable_id, "Stream 0 outbound command channel closed");
                        command_rx_open = false;
                    }
                }
            }
            read_res = read_control_packet(&mut recv_stream) => {
                match read_res {
                    Ok(packet) => {
                        // Reset heartbeat timeout on any valid received packet
                        heartbeat_sleep.as_mut().reset(tokio::time::Instant::now() + HEARTBEAT_TIMEOUT);

                        match packet {
                            ControlPacket::Ping { timestamp } => {
                                tracing::trace!(stable_id, timestamp, "Stream 0 received Ping, replying with Pong");
                                if let Some(ref mut session) = *state.session.write().await {
                                    session.touch_heartbeat();
                                }
                                let pong = ControlPacket::Pong { timestamp };
                                if let Err(e) = write_control_packet(&mut send_stream, &pong).await {
                                    tracing::warn!(stable_id, error = %e, "Failed to send Pong on Stream 0");
                                    state.clear_session(stable_id).await;
                                    return Err(e);
                                }
                            }
                            ControlPacket::Pong { timestamp } => {
                                tracing::trace!(stable_id, timestamp, "Stream 0 received Pong");
                                if let Some(ref mut session) = *state.session.write().await {
                                    session.touch_heartbeat();
                                }
                            }
                            ControlPacket::DeviceStatus {
                                battery_pct,
                                is_charging,
                                wifi_ssid,
                                cellular_signal,
                                hotspot_active,
                            } => {
                                tracing::debug!(
                                    stable_id,
                                    battery_pct,
                                    is_charging,
                                    wifi = ?wifi_ssid,
                                    cellular_signal,
                                    hotspot_active,
                                    "Stream 0 received DeviceStatus telemetry update"
                                );
                                let status = DeviceStatus {
                                    battery_pct,
                                    is_charging,
                                    wifi_ssid,
                                    cellular_signal,
                                    hotspot_active,
                                };
                                if let Some(ref mut session) = *state.session.write().await {
                                    session.update_status(status);
                                }
                            }
                            ControlPacket::Command { action } => {
                                tracing::debug!(stable_id, ?action, "Stream 0 received Command");
                                if let Some(ref mut session) = *state.session.write().await {
                                    session.touch_heartbeat();
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::info!(stable_id, error = %e, "Stream 0 disconnected or errored, clearing session");
                        state.clear_session(stable_id).await;
                        return Err(e);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quic_stream0_constants() {
        assert_eq!(HEARTBEAT_TIMEOUT, Duration::from_secs(5));
        assert_eq!(MAX_CONTROL_PACKET_LEN, 64 * 1024);
    }

    #[test]
    fn test_quic_stream0_packet_msgpack_and_json_compatibility() {
        let status = ControlPacket::DeviceStatus {
            battery_pct: 95,
            is_charging: true,
            wifi_ssid: Some("OmniLink-5G".to_string()),
            cellular_signal: 4,
            hotspot_active: false,
        };

        let msgpack_bytes = status.to_msgpack().expect("Failed to serialize to msgpack");
        let decoded_msgpack =
            ControlPacket::from_msgpack(&msgpack_bytes).expect("Failed to decode msgpack");
        assert_eq!(status, decoded_msgpack);

        let json_str = status.to_json().expect("Failed to serialize to json");
        let decoded_json = ControlPacket::from_json(&json_str).expect("Failed to decode json");
        assert_eq!(status, decoded_json);
    }
}
