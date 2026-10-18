//! Stream 1 QUIC clipboard transport pump, serialization, and loop prevention.

use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use quinn::{RecvStream, SendStream};

use omnilink_protocol::packet::ClipboardPacket;
use omnilink_protocol::error::ProtocolError;

use crate::clipboard::{ClipboardEvent, MAX_CLIPBOARD_PAYLOAD_SIZE};
use crate::quic::QuicError;
use crate::state::DaemonState;

/// Maximum payload size accepted for Stream 1 framed clipboard packets (20 MiB safety clamp).
pub const MAX_CLIPBOARD_FRAME_LEN: usize = MAX_CLIPBOARD_PAYLOAD_SIZE;

/// Reads a single framed `ClipboardPacket` from a QUIC receive stream.
///
/// Frame format: `[len: 4 bytes BE u32][encoded ClipboardPacket: len bytes]`.
/// Safety clamp: enforces payload size <= 20 MiB (`MAX_CLIPBOARD_FRAME_LEN`).
pub async fn read_clipboard_packet(
    recv_stream: &mut RecvStream,
) -> Result<ClipboardPacket, QuicError> {
    let mut len_buf = [0u8; 4];
    match recv_stream.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => {
            return Err(QuicError::ConnectionClosed(
                "Stream 1 closed by peer".to_string(),
            ));
        }
        Err(e) => return Err(QuicError::ReadExact(e)),
    }

    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len > MAX_CLIPBOARD_FRAME_LEN {
        return Err(QuicError::Protocol(ProtocolError::InvalidHeader(format!(
            "Clipboard packet exceeds maximum size limit ({} > {})",
            payload_len, MAX_CLIPBOARD_FRAME_LEN
        ))));
    }

    let mut buf = vec![0u8; payload_len];
    if payload_len > 0 {
        match recv_stream.read_exact(&mut buf).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => {
                return Err(QuicError::ConnectionClosed(
                    "Truncated clipboard packet received on Stream 1".to_string(),
                ));
            }
            Err(e) => return Err(QuicError::ReadExact(e)),
        }
    }

    let packet = ClipboardPacket::decode_binary(&buf)?;
    Ok(packet)
}

/// Encodes and writes a framed `ClipboardPacket` to a QUIC send stream.
///
/// Frame format: `[len: 4 bytes BE u32][encoded ClipboardPacket: len bytes]`.
/// Combines the 4-byte length prefix and binary packet into a single buffer to prevent fragmentation,
/// then flushes the stream.
pub async fn write_clipboard_packet(
    send_stream: &mut SendStream,
    packet: &ClipboardPacket,
) -> Result<(), QuicError> {
    let encoded = packet.encode_binary()?;
    let len = u32::try_from(encoded.len()).map_err(|_| {
        QuicError::Protocol(ProtocolError::Serialization(
            "Clipboard packet exceeds 4GB length limit".to_string(),
        ))
    })?;

    let mut buf = Vec::with_capacity(4 + encoded.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&encoded);

    send_stream.write_all(&buf).await?;
    send_stream.flush().await?;

    Ok(())
}

/// Handles the bidirectional Stream 1 clipboard transport pump for an authenticated connection.
///
/// Outbound pump:
/// - Receives local `ClipboardEvent`s from `clipboard_rx`.
/// - Wraps each event in a `ClipboardPacket` with a monotonically increasing sequence ID.
/// - Encodes to binary wire format, prepends 4-byte BE length prefix, and writes/flushes to `send`.
///
/// Inbound pump:
/// - Reads framed binary packets with a 20 MiB safety clamp.
/// - Decodes `ClipboardPacket::decode_binary`.
/// - Deduplicates via `state.clipboard_filter`: if `should_sync` is true, updates state and notifies local listeners.
/// - Gracefully shuts down when either stream ends or peer disconnects.
pub async fn run_stream_1(
    mut send: SendStream,
    mut recv: RecvStream,
    state: Arc<DaemonState>,
    mut clipboard_rx: mpsc::Receiver<ClipboardEvent>,
) -> Result<(), QuicError> {
    tracing::info!("Stream 1 clipboard sync pump started");

    let state_inbound = state.clone();

    let outbound_pump = async {
        let mut sequence_id: u64 = 1;
        while let Some(event) = clipboard_rx.recv().await {
            tracing::debug!(
                sequence_id,
                mime = %event.mime_type,
                is_sensitive = event.is_sensitive,
                bytes = event.payload.len(),
                "Stream 1 dispatching outbound clipboard event"
            );

            // Record into local filter to avoid echoes if local listener re-catches
            {
                let mut filter = state.clipboard_filter.lock().unwrap();
                filter.record(&event.payload);
            }

            let packet = ClipboardPacket {
                sequence_id,
                mime_type: event.mime_type,
                is_sensitive: event.is_sensitive,
                payload: event.payload,
            };
            sequence_id = sequence_id.wrapping_add(1);

            if let Err(e) = write_clipboard_packet(&mut send, &packet).await {
                tracing::warn!(error = %e, "Failed to write outbound clipboard packet on Stream 1");
                return Err(e);
            }
        }

        tracing::debug!("Stream 1 outbound clipboard channel closed, finishing send stream");
        let _ = send.finish();
        Ok::<(), QuicError>(())
    };

    let inbound_pump = async {
        loop {
            match read_clipboard_packet(&mut recv).await {
                Ok(packet) => {
                    let should_sync = {
                        let mut filter = state_inbound.clipboard_filter.lock().unwrap();
                        filter.should_sync(&packet.payload)
                    };

                    if should_sync {
                        tracing::debug!(
                            sequence_id = packet.sequence_id,
                            mime = %packet.mime_type,
                            is_sensitive = packet.is_sensitive,
                            bytes = packet.payload.len(),
                            "Stream 1 accepted inbound clipboard packet"
                        );
                        state_inbound.notify_inbound_clipboard(packet).await;
                    } else {
                        tracing::debug!(
                            sequence_id = packet.sequence_id,
                            "Stream 1 dropped duplicate inbound clipboard packet (echo loop prevention)"
                        );
                    }
                }
                Err(QuicError::ConnectionClosed(reason)) => {
                    tracing::debug!(%reason, "Stream 1 inbound closed by peer");
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Stream 1 inbound pump error");
                    return Err(e);
                }
            }
        }
        Ok::<(), QuicError>(())
    };

    tokio::select! {
        res = outbound_pump => {
            tracing::info!("Stream 1 outbound pump exited");
            res
        }
        res = inbound_pump => {
            tracing::info!("Stream 1 inbound pump exited");
            res
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn test_stream1_constants() {
        assert_eq!(MAX_CLIPBOARD_FRAME_LEN, 20 * 1024 * 1024);
        assert_eq!(MAX_CLIPBOARD_FRAME_LEN, MAX_CLIPBOARD_PAYLOAD_SIZE);
    }

    #[test]
    fn test_stream1_packet_framing_roundtrip() {
        let packet = ClipboardPacket {
            sequence_id: 42,
            mime_type: "text/plain;charset=utf-8".to_string(),
            is_sensitive: false,
            payload: b"Hello OmniLink Clipboard!".to_vec(),
        };

        let encoded = packet.encode_binary().expect("Encoding should succeed");
        let len = encoded.len() as u32;

        let mut framed = Vec::with_capacity(4 + encoded.len());
        framed.extend_from_slice(&len.to_be_bytes());
        framed.extend_from_slice(&encoded);

        // Verify length prefix
        let prefix = u32::from_be_bytes([framed[0], framed[1], framed[2], framed[3]]);
        assert_eq!(prefix as usize, encoded.len());

        let decoded = ClipboardPacket::decode_binary(&framed[4..]).expect("Decoding should succeed");
        assert_eq!(decoded.sequence_id, 42);
        assert_eq!(decoded.mime_type, "text/plain;charset=utf-8");
        assert!(!decoded.is_sensitive);
        assert_eq!(decoded.payload, b"Hello OmniLink Clipboard!");
    }

    #[test]
    fn test_stream1_sensitive_flag_roundtrip() {
        let packet = ClipboardPacket {
            sequence_id: 101,
            mime_type: "text/plain".to_string(),
            is_sensitive: true,
            payload: b"my-master-password-12345".to_vec(),
        };

        let encoded = packet.encode_binary().expect("Encoding should succeed");
        let decoded = ClipboardPacket::decode_binary(&encoded).expect("Decoding should succeed");
        assert_eq!(decoded.sequence_id, 101);
        assert!(decoded.is_sensitive);
        assert_eq!(decoded.payload, b"my-master-password-12345");
    }

    #[tokio::test]
    async fn test_stream1_state_and_filter_integration() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let state = Arc::new(DaemonState::new(signing_key, None));

        let payload = b"Desktop selection snippet 0xCAFE".to_vec();
        let packet = ClipboardPacket {
            sequence_id: 1,
            mime_type: "text/plain".to_string(),
            is_sensitive: false,
            payload: payload.clone(),
        };

        // First arrival should pass filter and notify state
        let should_sync_1 = {
            let mut filter = state.clipboard_filter.lock().unwrap();
            filter.should_sync(&packet.payload)
        };
        assert!(should_sync_1, "First occurrence must be accepted by filter");

        state.notify_inbound_clipboard(packet.clone()).await;

        let last_opt = state.last_received_clipboard().await;
        assert!(last_opt.is_some());
        let last = last_opt.unwrap();
        assert_eq!(last.sequence_id, 1);
        assert_eq!(last.payload, payload);

        // Second arrival with identical payload must be dropped by filter (echo loop prevention)
        let should_sync_2 = {
            let mut filter = state.clipboard_filter.lock().unwrap();
            filter.should_sync(&packet.payload)
        };
        assert!(!should_sync_2, "Duplicate occurrence must be rejected by filter");
    }
}
