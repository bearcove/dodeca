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

use crate::CellContext;

/// WebSocket handler - opens tunnel to host and pipes bytes
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<CellContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, ctx))
}

async fn handle_socket(socket: WebSocket, ctx: Arc<CellContext>) {
    // Open a WebSocket tunnel to the host
    let handle = match ctx.ws_tunnel_client().open().await {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to open WebSocket tunnel to host: {:?}", e);
            return;
        }
    };

    let channel_id = handle.channel_id;
    tracing::debug!(channel_id, "WebSocket tunnel opened to host");

    // Register to receive chunks from host
    let mut tunnel_rx = ctx.session.register_tunnel(channel_id);

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let session = ctx.session.clone();

    // Task A: WebSocket → Host (browser sends, we forward to tunnel)
    let session_a = session.clone();
    tokio::spawn(async move {
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Binary(data)) => {
                    if let Err(e) = session_a.send_chunk(channel_id, data).await {
                        tracing::debug!(channel_id, error = %e, "tunnel send error");
                        break;
                    }
                }
                Ok(Message::Text(text)) => {
                    // Send text as bytes too
                    let bytes = text.as_bytes().to_vec();
                    if let Err(e) = session_a.send_chunk(channel_id, bytes.into()).await {
                        tracing::debug!(channel_id, error = %e, "tunnel send error");
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => {
                    let _ = session_a.close_tunnel(channel_id).await;
                    break;
                }
                _ => {}
            }
        }
        tracing::debug!(channel_id, "WebSocket→Host task finished");
    });

    // Task B: Host → WebSocket (host sends, we forward to browser)
    tokio::spawn(async move {
        while let Some(chunk) = tunnel_rx.recv().await {
            let payload = chunk.payload_bytes();
            if !payload.is_empty() {
                // Send as binary (the devtools protocol uses binary postcard)
                if ws_sender
                    .send(Message::Binary(payload.to_vec().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            if chunk.is_eos() {
                tracing::debug!(channel_id, "received EOS from host");
                let _ = ws_sender.close().await;
                break;
            }
        }
        tracing::debug!(channel_id, "Host→WebSocket task finished");
    });
}
