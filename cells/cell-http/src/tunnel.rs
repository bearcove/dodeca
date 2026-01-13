//! TCP tunnel implementation for the cell.
//!
//! Implements the `TcpTunnel` service that the host calls to open tunnels.
//! Each tunnel serves HTTP directly using roam's tunnel streaming.

use std::sync::Arc;
use std::time::Instant;

use roam::{DEFAULT_TUNNEL_CHUNK_SIZE, Tunnel, tunnel_stream};

use cell_http_proto::TcpTunnel;

use crate::RouterContext;

/// Cell-side implementation of `TcpTunnel`.
///
/// Each `open()` call receives a tunnel from the host and serves HTTP on it.
#[derive(Clone)]
pub struct TcpTunnelImpl {
    #[allow(dead_code)]
    ctx: Arc<dyn RouterContext>,
    app: axum::Router,
}

impl TcpTunnelImpl {
    pub fn new(ctx: Arc<dyn RouterContext>, app: axum::Router) -> Self {
        Self { ctx, app }
    }
}

impl TcpTunnel for TcpTunnelImpl {
    async fn open(&self, tunnel: Tunnel) {
        let channel_id = tunnel.tx.channel_id();
        tracing::info!(channel_id, "HTTP tunnel opened");

        let service = self.app.clone();

        // Create a duplex stream to bridge the tunnel to hyper
        let (client, server) = tokio::io::duplex(64 * 1024);

        // Spawn tasks to pump data between tunnel and duplex
        let (read_handle, write_handle) = tunnel_stream(client, tunnel, DEFAULT_TUNNEL_CHUNK_SIZE);

        // Serve HTTP on the server side of the duplex
        tokio::spawn(async move {
            let started_at = Instant::now();
            tracing::info!(channel_id, "HTTP connection starting");
            if let Err(e) = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    hyper_util::rt::TokioIo::new(server),
                    hyper_util::service::TowerToHyperService::new(service),
                )
                // Enable WebSocket upgrades - keeps connection alive after 101 response
                .with_upgrades()
                .await
            {
                tracing::warn!(
                    channel_id,
                    error = %e,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    "HTTP connection error"
                );
            }
            tracing::info!(
                channel_id,
                elapsed_ms = started_at.elapsed().as_millis(),
                "HTTP connection finished"
            );

            // Wait for tunnel pumps to finish and log any errors
            match read_handle.await {
                Ok(Ok(())) => tracing::debug!(channel_id, "tunnel read pump completed"),
                Ok(Err(e)) => tracing::warn!(channel_id, error = %e, "tunnel read pump error"),
                Err(e) => tracing::warn!(channel_id, error = %e, "tunnel read pump task panicked"),
            }
            match write_handle.await {
                Ok(Ok(())) => tracing::debug!(channel_id, "tunnel write pump completed"),
                Ok(Err(e)) => tracing::warn!(channel_id, error = %e, "tunnel write pump error"),
                Err(e) => tracing::warn!(channel_id, error = %e, "tunnel write pump task panicked"),
            }
        });
    }
}
