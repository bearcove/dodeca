//! Devtools WebSocket handler
//!
//! Handles the /_/ws endpoint by opening a tunnel to the host and piping
//! WebSocket frames through it. The cell doesn't understand the devtools
//! protocol - it just bridges bytes.

use std::sync::Arc;

use axum::extract::{
    State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use roam::tunnel_pair;

use crate::RouterContext;

/// WebSocket handler - opens tunnel to host and pipes bytes
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<dyn RouterContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, ctx))
}

async fn handle_socket(socket: WebSocket, ctx: Arc<dyn RouterContext>) {
    tracing::debug!("DevTools WebSocket connection received, opening tunnel to host...");

    // Create a tunnel pair - local stays here, remote goes to host
    let (local, remote) = tunnel_pair();
    let channel_id = local.tx.channel_id();

    // Open a WebSocket tunnel to the host, passing the remote end
    if let Err(e) = ctx.host_client().open_websocket(remote).await {
        tracing::error!("Failed to open WebSocket tunnel to host: {:?}", e);
        return;
    }

    tracing::debug!(channel_id, "WebSocket tunnel opened to host");

    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Decompose the local tunnel
    let local_tx = local.tx;
    let mut local_rx = local.rx;

    // Task A: WebSocket → Host (browser sends, we forward to tunnel)
    let ws_to_host = tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    if local_tx.send(&data.to_vec()).await.is_err() {
                        tracing::debug!(channel_id, "tunnel send error");
                        break;
                    }
                }
                Ok(Message::Text(text)) => {
                    // Send text as bytes too
                    if local_tx.send(&text.as_bytes().to_vec()).await.is_err() {
                        tracing::debug!(channel_id, "tunnel send error");
                        break;
                    }
                }
                Ok(Message::Close(frame)) => {
                    tracing::debug!(channel_id, ?frame, "WebSocket close frame received");
                    break;
                }
                Err(e) => {
                    tracing::warn!(channel_id, error = %e, "WebSocket receive error");
                    break;
                }
                _ => {}
            }
        }
        tracing::debug!(channel_id, "WebSocket→Host task finished");
    });

    // Task B: Host → WebSocket (host sends, we forward to browser)
    let host_to_ws = tokio::spawn(async move {
        loop {
            match local_rx.recv().await {
                Ok(Some(data)) => {
                    if !data.is_empty() {
                        tracing::trace!(
                            channel_id,
                            bytes = data.len(),
                            "Received data from host, forwarding to WebSocket"
                        );
                        // Send as binary (the devtools protocol uses binary postcard)
                        if ws_sender.send(Message::Binary(data.into())).await.is_err() {
                            tracing::debug!(channel_id, "WebSocket send failed, closing");
                            break;
                        }
                    }
                }
                Ok(None) => {
                    tracing::debug!(channel_id, "Host channel closed (None)");
                    break;
                }
                Err(e) => {
                    tracing::warn!(channel_id, error = ?e, "Host channel recv error");
                    break;
                }
            }
        }
        tracing::debug!(channel_id, "Host→WebSocket task finished");
        if let Err(e) = ws_sender.close().await {
            tracing::debug!(channel_id, error = %e, "WebSocket close error (may be already closed)");
        }
    });

    // Wait for both tasks to complete
    let (ws_to_host_result, host_to_ws_result) = tokio::join!(ws_to_host, host_to_ws);
    if let Err(e) = ws_to_host_result {
        tracing::warn!(channel_id, error = %e, "WebSocket→Host task panicked");
    }
    if let Err(e) = host_to_ws_result {
        tracing::warn!(channel_id, error = %e, "Host→WebSocket task panicked");
    }
    tracing::debug!(channel_id, "DevTools WebSocket handler finished");
}
