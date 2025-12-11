//! TCP tunnel implementation for the plugin.
//!
//! Implements the TcpTunnel service that the host calls to open tunnels.
//! Each tunnel bridges a rapace channel with a TCP connection to the
//! internal HTTP server.

use std::sync::Arc;

use rapace::{RpcSession, Transport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use mod_http_proto::{TcpTunnel, TunnelHandle};

/// Default buffer size for reads (4KB chunks).
pub const CHUNK_SIZE: usize = 4096;

/// Plugin-side implementation of TcpTunnel.
///
/// Each `open()` call:
/// 1. Allocates a new channel_id
/// 2. Connects to the internal HTTP server
/// 3. Spawns tasks to bridge rapace ↔ TCP
pub struct TcpTunnelImpl<T: Transport> {
    session: Arc<RpcSession<T>>,
    internal_port: u16,
}

impl<T: Transport + Send + Sync + 'static> TcpTunnelImpl<T> {
    pub fn new(session: Arc<RpcSession<T>>, internal_port: u16) -> Self {
        Self {
            session,
            internal_port,
        }
    }
}

impl<T: Transport + Send + Sync + 'static> TcpTunnel for TcpTunnelImpl<T> {
    async fn open(&self) -> TunnelHandle {
        // Allocate a channel for this tunnel
        let channel_id = self.session.next_channel_id();

        tracing::debug!(channel_id, "tunnel open requested");

        // Register the tunnel to receive incoming chunks from host
        let mut tunnel_rx = self.session.register_tunnel(channel_id);

        // Connect to the internal HTTP server
        let addr = format!("127.0.0.1:{}", self.internal_port);
        let tcp_stream = match TcpStream::connect(&addr).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(channel_id, error = %e, "failed to connect to internal HTTP server");
                // Return the handle anyway - the tunnel tasks will fail gracefully
                return TunnelHandle { channel_id };
            }
        };

        let (mut tcp_read, mut tcp_write) = tcp_stream.into_split();
        let session = self.session.clone();

        // Task A: rapace → TCP (read from tunnel, write to TCP socket)
        tokio::spawn(async move {
            while let Some(chunk) = tunnel_rx.recv().await {
                if !chunk.payload.is_empty()
                    && let Err(e) = tcp_write.write_all(&chunk.payload).await
                {
                    tracing::debug!(channel_id, error = %e, "TCP write error");
                    break;
                }
                if chunk.is_eos {
                    tracing::debug!(channel_id, "received EOS from host");
                    // Half-close the TCP write side
                    let _ = tcp_write.shutdown().await;
                    break;
                }
            }
            tracing::debug!(channel_id, "rapace→TCP task finished");
        });

        // Task B: TCP → rapace (read from TCP socket, write to tunnel)
        let session_b = session.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; CHUNK_SIZE];
            loop {
                match tcp_read.read(&mut buf).await {
                    Ok(0) => {
                        // TCP EOF - close the tunnel
                        tracing::debug!(channel_id, "TCP EOF, closing tunnel");
                        let _ = session_b.close_tunnel(channel_id).await;
                        break;
                    }
                    Ok(n) => {
                        if let Err(e) = session_b.send_chunk(channel_id, buf[..n].to_vec()).await {
                            tracing::debug!(channel_id, error = %e, "tunnel send error");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(channel_id, error = %e, "TCP read error");
                        let _ = session_b.close_tunnel(channel_id).await;
                        break;
                    }
                }
            }
            tracing::debug!(channel_id, "TCP→rapace task finished");
        });

        TunnelHandle { channel_id }
    }
}

// Need to implement Clone for TcpTunnelImpl to use with the server
impl<T: Transport + Send + Sync + 'static> Clone for TcpTunnelImpl<T> {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            internal_port: self.internal_port,
        }
    }
}
