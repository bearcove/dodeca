//! Devtools WebSocket handler.
//!
//! TODO(devtools-forwarding): the roam version ran a roam RPC session on the
//! browser WebSocket and forwarded it to the host via
//! `roam_session::ForwardingDispatcher` + `LateBoundForwarder` over a virtual
//! connection (`ctx.handle().connect(...)`). The vox rebuild is a
//! WS-backed `vox` link proxied to a host connection (`vox::proxy_connections`
//! / `PendingConnection::proxy_to`), paired with the per-vconn
//! `HostDevtoolsService` browser-identity work that is deferred until the
//! workspace builds green. Until then `/_/ws` is accepted and closed so the
//! cell compiles and content serving works; browser devtools/livereload are
//! inactive.

use std::sync::Arc;

use axum::extract::{State, WebSocketUpgrade, ws::WebSocket};
use axum::response::IntoResponse;

use crate::RouterContext;

/// WebSocket handler — devtools forwarding not yet rebuilt on vox.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(_ctx): State<Arc<dyn RouterContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket: WebSocket| async move {
        tracing::debug!(
            "devtools WebSocket received but forwarding is not yet rebuilt on vox; closing"
        );
        drop(socket);
    })
}
