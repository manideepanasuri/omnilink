//! Stream 2 QUIC notification transport pump, framing, and bidirectional synchronization.

use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use quinn::{RecvStream, SendStream};

use omnilink_protocol::error::ProtocolError;
use omnilink_protocol::packet::NotificationStreamPacket;

use crate::quic::QuicError;
use crate::state::DaemonState;

/// Maximum payload size accepted for Stream 2 framed notification packets (20 MiB safety clamp).
pub const MAX_NOTIFICATION_FRAME_LEN: usize = 20 * 1024 * 1024;

/// Reads a single framed `NotificationStreamPacket` from a QUIC receive stream.
///
/// Frame format: `[len: 4 bytes BE u32][payload: len bytes]`.
/// Safety clamp: enforces payload size <= 20 MiB (`MAX_NOTIFICATION_FRAME_LEN`).
pub async fn read_notification_packet(
    recv_stream: &mut RecvStream,
) -> Result<NotificationStreamPacket, QuicError> {
    let mut len_buf = [0u8; 4];
    match recv_stream.read_exact(&mut len_buf).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => {
            return Err(QuicError::ConnectionClosed(
                "Stream 2 closed by peer".to_string(),
            ));
        }
        Err(e) => return Err(QuicError::ReadExact(e)),
    }

    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len > MAX_NOTIFICATION_FRAME_LEN {
        return Err(QuicError::Protocol(ProtocolError::InvalidHeader(format!(
            "Notification packet exceeds maximum size limit ({} > {})",
            payload_len, MAX_NOTIFICATION_FRAME_LEN
        ))));
    }

    let mut buf = vec![0u8; payload_len];
    if payload_len > 0 {
        match recv_stream.read_exact(&mut buf).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => {
                return Err(QuicError::ConnectionClosed(
                    "Truncated notification packet received on Stream 2".to_string(),
                ));
            }
            Err(e) => return Err(QuicError::ReadExact(e)),
        }
    }

    let packet = NotificationStreamPacket::decode_binary(&buf)?;
    Ok(packet)
}

/// Encodes and writes a framed `NotificationStreamPacket` to a QUIC send stream.
///
/// Frame format: `[len: 4 bytes BE u32][payload: len bytes]`.
/// Combines the 4-byte length prefix and encoded payload into a single buffer to prevent fragmentation,
/// then flushes the stream.
pub async fn write_notification_packet(
    send_stream: &mut SendStream,
    packet: &NotificationStreamPacket,
) -> Result<(), QuicError> {
    let encoded = packet.encode_binary()?;
    let len = u32::try_from(encoded.len()).map_err(|_| {
        QuicError::Protocol(ProtocolError::Serialization(
            "Notification packet exceeds 4GB length limit".to_string(),
        ))
    })?;

    let mut buf = Vec::with_capacity(4 + encoded.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&encoded);

    send_stream.write_all(&buf).await?;
    send_stream.flush().await?;

    Ok(())
}

/// Handles the bidirectional Stream 2 notification transport pump for an authenticated connection.
///
/// Outbound pump:
/// - Receives `NotificationStreamPacket`s (replies or desktop dismissals) from `notification_rx`.
/// - Encodes to binary wire format, prepends 4-byte BE length prefix, and writes/flushes to `send`.
///
/// Inbound pump:
/// - Reads framed packets with a 20 MiB safety clamp.
/// - Decodes `NotificationStreamPacket::decode_binary`.
/// - Dispatches:
///   - `Posted`: forwards to Linux desktop via D-Bus (`state.notification_forwarder`) and broadcasts to local listeners.
///   - `Dismiss`: closes corresponding desktop notification on Linux via D-Bus and broadcasts to local listeners.
///   - `Reply`: broadcasts reply event to local listeners.
/// - Gracefully shuts down when either stream ends or peer disconnects.
pub async fn run_stream_2(
    mut send: SendStream,
    mut recv: RecvStream,
    state: Arc<DaemonState>,
    mut notification_rx: mpsc::Receiver<NotificationStreamPacket>,
) -> Result<(), QuicError> {
    tracing::info!("Stream 2 notification sync pump started");

    let state_inbound = state.clone();

    let outbound_pump = async {
        while let Some(packet) = notification_rx.recv().await {
            tracing::debug!("Stream 2 dispatching outbound notification stream packet");
            if let Err(e) = write_notification_packet(&mut send, &packet).await {
                tracing::warn!(error = %e, "Failed to write outbound notification packet on Stream 2");
                return Err(e);
            }
        }

        tracing::debug!("Stream 2 outbound notification channel closed, finishing send stream");
        let _ = send.finish();
        Ok::<(), QuicError>(())
    };

    let inbound_pump = async {
        loop {
            match read_notification_packet(&mut recv).await {
                Ok(packet) => match packet {
                    NotificationStreamPacket::Posted(posted) => {
                        tracing::debug!(
                            id = %posted.id,
                            app = %posted.app_name,
                            title = %posted.title,
                            can_reply = posted.can_reply,
                            "Stream 2 received inbound posted notification"
                        );

                        let forwarder = state_inbound.notification_forwarder.read().await.clone();
                        if let Some(forwarder) = forwarder {
                            if let Err(e) = forwarder.forward_notification(&posted).await {
                                tracing::warn!(id = %posted.id, error = %e, "Failed to forward notification to D-Bus");
                            }
                        }

                        state_inbound.notify_inbound_notification(posted).await;
                    }
                    NotificationStreamPacket::Dismiss(dismiss) => {
                        tracing::debug!(
                            id = %dismiss.id,
                            "Stream 2 received inbound notification dismissal from companion"
                        );

                        let forwarder = state_inbound.notification_forwarder.read().await.clone();
                        if let Some(forwarder) = forwarder {
                            if let Err(e) = forwarder.close_desktop_notification(&dismiss.id).await {
                                tracing::warn!(id = %dismiss.id, error = %e, "Failed to close desktop notification from companion dismissal");
                            }
                        }

                        state_inbound.notify_inbound_dismiss(dismiss).await;
                    }
                    NotificationStreamPacket::Reply(reply) => {
                        tracing::debug!(
                            id = %reply.notification_id,
                            action = %reply.action_key,
                            "Stream 2 received inbound notification reply packet"
                        );

                        state_inbound.notify_inbound_reply(reply).await;
                    }
                },
                Err(QuicError::ConnectionClosed(reason)) => {
                    tracing::debug!(%reason, "Stream 2 inbound closed by peer");
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Stream 2 inbound pump error");
                    return Err(e);
                }
            }
        }

        Ok::<(), QuicError>(())
    };

    tokio::select! {
        res = outbound_pump => {
            if let Err(e) = res {
                tracing::warn!(error = %e, "Stream 2 outbound pump terminated with error");
                return Err(e);
            }
        }
        res = inbound_pump => {
            if let Err(e) = res {
                tracing::warn!(error = %e, "Stream 2 inbound pump terminated with error");
                return Err(e);
            }
        }
    }

    tracing::info!("Stream 2 notification sync pump completed");
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use omnilink_protocol::packet::{
        NotificationAction, NotificationDismissPacket, NotificationPacket,
        NotificationReplyPacket,
    };

    #[test]
    fn test_stream2_constants() {
        assert_eq!(MAX_NOTIFICATION_FRAME_LEN, 20 * 1024 * 1024);
    }

    #[test]
    fn test_stream2_packet_roundtrips() {
        let posted = NotificationStreamPacket::Posted(NotificationPacket {
            id: "id-1".to_string(),
            package_name: "pkg".to_string(),
            app_name: "App".to_string(),
            title: "Title".to_string(),
            text: "Text".to_string(),
            icon_png: None,
            timestamp: 100,
            can_reply: true,
            actions: vec![NotificationAction {
                key: "key".to_string(),
                title: "Reply".to_string(),
                is_input: true,
            }],
        });
        let bytes = posted.encode_binary().expect("encode posted");
        let decoded = NotificationStreamPacket::decode_binary(&bytes).expect("decode posted");
        assert_eq!(posted, decoded);

        let reply = NotificationStreamPacket::Reply(NotificationReplyPacket {
            notification_id: "id-1".to_string(),
            action_key: "inline-reply".to_string(),
            reply_text: "Replying from desktop".to_string(),
        });
        let bytes = reply.encode_binary().expect("encode reply");
        let decoded = NotificationStreamPacket::decode_binary(&bytes).expect("decode reply");
        assert_eq!(reply, decoded);

        let dismiss = NotificationStreamPacket::Dismiss(NotificationDismissPacket {
            id: "id-1".to_string(),
        });
        let bytes = dismiss.encode_binary().expect("encode dismiss");
        let decoded = NotificationStreamPacket::decode_binary(&bytes).expect("decode dismiss");
        assert_eq!(dismiss, decoded);
    }
}
