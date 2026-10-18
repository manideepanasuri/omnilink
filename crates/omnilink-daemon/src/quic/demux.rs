//! QUIC stream demultiplexer routing dedicated bidirectional and unidirectional streams.

use std::sync::Arc;

use quinn::Connection;

use crate::quic::stream0::run_stream_0;
use crate::quic::stream1::run_stream_1;
use crate::quic::QuicError;
use crate::state::{ActiveSession, DaemonState};

/// Handles an incoming authenticated QUIC connection, establishing the active session
/// and demultiplexing dedicated streams according to the protocol specification.
pub async fn handle_connection(
    connection: Connection,
    state: Arc<DaemonState>,
    device_id: String,
) -> Result<(), QuicError> {
    let stable_id = connection.stable_id();
    tracing::info!(%device_id, stable_id, remote_addr = %connection.remote_address(), "Authenticated companion connected");

    let (command_tx, mut command_rx_opt) = {
        let (tx, rx) = tokio::sync::mpsc::channel(32);
        (tx, Some(rx))
    };

    let (clipboard_tx, mut clipboard_rx_opt) = {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        (tx, Some(rx))
    };

    // Initialize active session telemetry
    {
        let session = ActiveSession::new(
            device_id.clone(),
            connection.clone(),
            command_tx,
            clipboard_tx,
        );
        *state.session.write().await = Some(session);
    }

    let conn_clone = connection.clone();
    let state_clone = state.clone();

    // Stream demuxer loop
    let demux_result: Result<(), QuicError> = async {
        loop {
            tokio::select! {
                bi_res = connection.accept_bi() => {
                    match bi_res {
                        Ok((send_stream, recv_stream)) => {
                            let stream_index = send_stream.id().index();
                            tracing::debug!(index = stream_index, "Accepted bidirectional stream");

                            match stream_index {
                                0 => {
                                    // Stream 0: Control, Telemetry & Heartbeat
                                    let s_clone = state.clone();
                                    if let Some(command_rx) = command_rx_opt.take() {
                                        tokio::spawn(async move {
                                            if let Err(e) = run_stream_0(send_stream, recv_stream, s_clone, command_rx, stable_id).await {
                                                tracing::debug!(error = %e, "Stream 0 finished");
                                            }
                                        });
                                    } else {
                                        tracing::warn!("Stream 0 already established for this connection, rejecting duplicate");
                                    }
                                }
                                1 => {
                                    // Stream 1 (Bi): Bidirectional Clipboard Sync
                                    let s_clone = state.clone();
                                    if let Some(clipboard_rx) = clipboard_rx_opt.take() {
                                        tokio::spawn(async move {
                                            if let Err(e) = run_stream_1(send_stream, recv_stream, s_clone, clipboard_rx).await {
                                                tracing::debug!(error = %e, "Stream 1 finished");
                                            }
                                        });
                                    } else {
                                        tracing::warn!("Stream 1 already established for this connection, rejecting duplicate");
                                    }
                                }
                                2 => {
                                    // Stream 2 (Bi): Notifications & Action Replies (Extension point)
                                    tracing::info!("Accepted Stream 2 (Notifications) - ready for pump");
                                }
                                3 => {
                                    // Stream 3 (Bi): High-Throughput File & Photo Sync (Extension point)
                                    tracing::info!("Accepted Stream 3 (File Transfer) - ready for dispatcher");
                                }
                                other => {
                                    tracing::debug!(index = other, "Accepted unhandled bidirectional stream");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::info!(error = %e, "Connection closed while accepting bi stream");
                            return Err(QuicError::Connection(e));
                        }
                    }
                }
                uni_res = connection.accept_uni() => {
                    match uni_res {
                        Ok(recv_stream) => {
                            let stream_index = recv_stream.id().index();
                            tracing::debug!(index = stream_index, "Accepted unidirectional stream");

                            match stream_index {
                                0 => {
                                    // Stream 4 (Uni): Screen & App Mirroring Video Stream (Extension point)
                                    tracing::info!("Accepted Stream 4 (Video Stream)");
                                }
                                1 => {
                                    // Stream 5 (Uni): Remote Input Events (Extension point)
                                    tracing::info!("Accepted Stream 5 (Remote Input)");
                                }
                                other => {
                                    tracing::debug!(index = other, "Accepted unhandled unidirectional stream");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::info!(error = %e, "Connection closed while accepting uni stream");
                            return Err(QuicError::Connection(e));
                        }
                    }
                }
                _ = conn_clone.closed() => {
                    tracing::info!("QUIC connection closed by peer or transport error");
                    break;
                }
            }
        }
        Ok(())
    }
    .await;

    // Clear active session upon termination
    tracing::info!(%device_id, stable_id, "Demuxer finished, clearing active session");
    state_clone.clear_session(stable_id).await;

    demux_result
}
