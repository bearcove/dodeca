//! TCP tunnel implementation for the cell.
//!
//! Implements the `TcpTunnel` service the host calls to open tunnels. A tunnel
//! is a pair of vox channels: the host pumps the browser TCP socket into
//! `inbound` and reads responses from `outbound`; here we bridge those channels
//! to an in-cell hyper server running the axum router.

use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vox::{Rx, Tx};

use cell_http_proto::TcpTunnel;

use crate::RouterContext;

/// Cell-side implementation of `TcpTunnel`.
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
    async fn open(&self, mut inbound: Rx<Vec<u8>>, outbound: Tx<Vec<u8>>) {
        let service = self.app.clone();

        // Bridge the channel pair to hyper via an in-memory duplex.
        let (hyper_io, pump_io) = tokio::io::duplex(64 * 1024);
        let (mut pump_rd, mut pump_wr) = tokio::io::split(pump_io);

        tokio::spawn(async move {
            let started_at = Instant::now();
            tracing::trace!("HTTP tunnel connection starting");

            // Serve HTTP on the hyper side of the duplex.
            let serve = tokio::spawn(async move {
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        hyper_util::rt::TokioIo::new(hyper_io),
                        hyper_util::service::TowerToHyperService::new(service),
                    )
                    .with_upgrades()
                    .await
                {
                    tracing::warn!(error = %e, "HTTP connection error");
                }
            });

            // browser -> hyper: inbound channel into the duplex
            let up = async move {
                while let Ok(Some(chunk)) = inbound.recv().await {
                    // Vec<u8> isn't `Reborrow`; move the owned bytes out via map.
                    let mut bytes: Option<Vec<u8>> = None;
                    let _ = chunk.map(|v| {
                        bytes = Some(v);
                    });
                    let Some(bytes) = bytes else { break };
                    if pump_wr.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                let _ = pump_wr.shutdown().await;
            };

            // hyper -> browser: duplex out to the outbound channel
            let down = async move {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match pump_rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if outbound.send(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            };

            tokio::join!(up, down);
            let _ = serve.await;
            tracing::trace!(
                elapsed_ms = started_at.elapsed().as_millis(),
                "HTTP tunnel connection finished"
            );
        });
    }
}
