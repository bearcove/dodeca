//! TCP tunnel implementation for the cell.
//!
//! Implements the `TcpTunnel` service that the host calls to open tunnels.
//! Each tunnel serves HTTP directly on a first-class rapace `TunnelStream`.

use std::sync::Arc;

use rapace::RpcSession;

use cell_http_proto::{TcpTunnel, TunnelHandle};

/// Cell-side implementation of `TcpTunnel`.
///
/// Each `open()` call allocates a new tunnel channel and serves HTTP/1 on it.
pub struct TcpTunnelImpl {
    session: Arc<RpcSession>,
    app: axum::Router,
}

impl TcpTunnelImpl {
    pub fn new(session: Arc<RpcSession>, app: axum::Router) -> Self {
        Self { session, app }
    }
}

impl TcpTunnel for TcpTunnelImpl {
    async fn open(&self) -> TunnelHandle {
        let (handle, stream) = self.session.open_tunnel_stream();
        let channel_id = handle.channel_id;

        let service = self.app.clone();
        tokio::spawn(async move {
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    hyper_util::rt::TokioIo::new(stream),
                    hyper_util::service::TowerToHyperService::new(service),
                )
                .await
            {
                tracing::debug!(channel_id, error = %e, "HTTP connection error");
            }
            tracing::debug!(channel_id, "HTTP connection finished");
        });

        TunnelHandle { channel_id }
    }
}

impl Clone for TcpTunnelImpl {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            app: self.app.clone(),
        }
    }
}
