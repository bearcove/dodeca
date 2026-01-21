//! Devtools WebSocket handler
//!
//! Handles the /_/ws endpoint by running a roam RPC session on the WebSocket
//! and forwarding all calls to the host via ForwardingDispatcher.
//!
//! This allows browser-based devtools to call DevtoolsService methods
//! directly via roam RPC.

use std::sync::Arc;

use axum::extract::{
    State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use roam_session::{ForwardingDispatcher, LateBoundForwarder, LateBoundHandle};
use roam_stream::{HandshakeConfig, accept_framed};

use crate::RouterContext;

/// WebSocket handler - runs roam RPC and forwards to host
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<dyn RouterContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, ctx))
}

async fn handle_socket(socket: WebSocket, ctx: Arc<dyn RouterContext>) {
    tracing::debug!("DevTools WebSocket connection received, setting up RPC forwarding...");

    // Wrap the axum WebSocket in a transport adapter
    let transport = AxumWsTransport::new(socket);

    // Create a late-bound forwarder for Host → Browser direction.
    // We can't create this yet because we don't have the browser handle.
    let late_bound = LateBoundHandle::new();
    let host_to_browser = LateBoundForwarder::new(late_bound.clone());

    // Each browser gets its own virtual connection to the host.
    // Pass the late-bound forwarder so the host can push calls to the browser.
    let virtual_handle = match ctx
        .handle()
        .connect(
            roam_wire::Metadata::default(),
            Some(Box::new(host_to_browser)),
        )
        .await
    {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!("Failed to open virtual connection to host: {:?}", e);
            return;
        }
    };

    // Create forwarding dispatcher for Browser → Host direction (immediate)
    let browser_to_host = ForwardingDispatcher::new(virtual_handle);

    // Accept the roam session with the forwarding dispatcher
    let config = HandshakeConfig::default();
    match accept_framed(transport, config, browser_to_host).await {
        Ok((browser_handle, _incoming, driver)) => {
            // NOW we can bind the late-bound forwarder for Host → Browser
            late_bound.set(browser_handle);

            tracing::debug!("DevTools RPC session established (bidirectional forwarding active)");
            // Run the driver until the connection closes
            if let Err(e) = driver.run().await {
                tracing::warn!("DevTools RPC driver error: {:?}", e);
            }
            tracing::debug!("DevTools RPC session ended");
        }
        Err(e) => {
            tracing::error!("Failed to establish DevTools RPC session: {:?}", e);
        }
    }
}

// ============================================================================
// AxumWsTransport - Adapts axum WebSocket to MessageTransport
// ============================================================================

use roam_session::MessageTransport;
use roam_wire::Message as RoamMessage;
use std::io;
use std::time::Duration;

/// Adapter that implements MessageTransport for axum WebSocket.
///
/// This allows running a roam driver directly on an axum WebSocket connection.
struct AxumWsTransport {
    sender: futures_util::stream::SplitSink<WebSocket, Message>,
    receiver: futures_util::stream::SplitStream<WebSocket>,
    last_decoded: Vec<u8>,
}

impl AxumWsTransport {
    fn new(socket: WebSocket) -> Self {
        let (sender, receiver) = socket.split();
        Self {
            sender,
            receiver,
            last_decoded: Vec::new(),
        }
    }
}

impl MessageTransport for AxumWsTransport {
    async fn send(&mut self, msg: &RoamMessage) -> io::Result<()> {
        let payload = facet_postcard::to_vec(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        tracing::debug!(
            msg_type = ?std::mem::discriminant(msg),
            payload_len = payload.len(),
            payload_bytes = ?&payload[..payload.len().min(64)],
            "Sending WebSocket message to browser"
        );

        self.sender
            .send(Message::Binary(payload.into()))
            .await
            .map_err(|e| io::Error::other(format!("WebSocket send failed: {e}")))?;

        Ok(())
    }

    async fn recv_timeout(&mut self, timeout: Duration) -> io::Result<Option<RoamMessage>> {
        tokio::select! {
            result = self.recv() => result,
            _ = tokio::time::sleep(timeout) => Ok(None),
        }
    }

    async fn recv(&mut self) -> io::Result<Option<RoamMessage>> {
        loop {
            match self.receiver.next().await {
                Some(Ok(Message::Binary(data))) => {
                    self.last_decoded = data.to_vec();
                    let msg: RoamMessage =
                        facet_postcard::from_slice(&self.last_decoded).map_err(|e| {
                            io::Error::new(io::ErrorKind::InvalidData, format!("postcard: {e}"))
                        })?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Text(text))) => {
                    // Treat text as binary (shouldn't happen for roam protocol)
                    self.last_decoded = text.as_bytes().to_vec();
                    let msg: RoamMessage =
                        facet_postcard::from_slice(&self.last_decoded).map_err(|e| {
                            io::Error::new(io::ErrorKind::InvalidData, format!("postcard: {e}"))
                        })?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Close(_))) => {
                    return Ok(None);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                    // Ignore ping/pong, continue receiving
                    continue;
                }
                Some(Err(e)) => {
                    return Err(io::Error::other(format!("WebSocket error: {e}")));
                }
                None => {
                    // Stream ended
                    return Ok(None);
                }
            }
        }
    }

    fn last_decoded(&self) -> &[u8] {
        &self.last_decoded
    }
}
