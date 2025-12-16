//! TCP tunnel implementation for the plugin.
//!
//! Implements the TcpTunnel service that the host calls to open tunnels.
//! Each tunnel serves HTTP directly on the rapace channel stream.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use rapace::RpcSession;
use rapace::rapace_core::TunnelChunk;
use tokio::io::AsyncWrite;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::StreamReader;

use cell_http_proto::{TcpTunnel, TunnelHandle};

/// Plugin-side implementation of TcpTunnel.
///
/// Each `open()` call:
/// 1. Allocates a new channel_id
/// 2. Creates a duplex stream from rapace channels
/// 3. Serves HTTP directly on that stream with hyper
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
        // Allocate a channel for this tunnel
        let channel_id = self.session.next_channel_id();

        tracing::debug!(channel_id, "tunnel open requested");

        // Register the tunnel to receive incoming chunks from host
        let tunnel_rx = self.session.register_tunnel(channel_id);

        // Convert receiver to a Stream, then to AsyncRead using StreamReader
        fn chunk_to_bytes(chunk: TunnelChunk) -> Result<Bytes, std::io::Error> {
            if chunk.is_eos {
                Ok(Bytes::new()) // EOF
            } else {
                Ok(Bytes::from(chunk.payload))
            }
        }
        let rx_stream = ReceiverStream::new(tunnel_rx).map(chunk_to_bytes as fn(_) -> _);
        let reader = StreamReader::new(rx_stream);

        // Create AsyncWrite that sends to the session
        let writer = TunnelWriter {
            channel_id,
            session: self.session.clone(),
            pending_send: None,
            closed: false,
        };

        // Combine into a duplex stream
        let stream = TunnelStream { reader, writer };

        // Serve HTTP directly on the tunnel stream
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

// Need to implement Clone for TcpTunnelImpl to use with the server
impl Clone for TcpTunnelImpl {
    fn clone(&self) -> Self {
        Self {
            session: self.session.clone(),
            app: self.app.clone(),
        }
    }
}

/// Writer that sends data to the rapace session
#[allow(clippy::type_complexity)]
struct TunnelWriter {
    channel_id: u32,
    session: Arc<RpcSession>,
    pending_send:
        Option<Pin<Box<dyn std::future::Future<Output = Result<(), rapace::RpcError>> + Send>>>,
    closed: bool,
}

impl AsyncWrite for TunnelWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "tunnel closed",
            )));
        }

        // If there's a pending send, poll it first
        if let Some(fut) = self.pending_send.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => {
                    self.pending_send = None;
                }
                Poll::Ready(Err(e)) => {
                    self.pending_send = None;
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        format!("send failed: {e:?}"),
                    )));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }

        // Start a new send
        let channel_id = self.channel_id;
        let session = self.session.clone();
        let data = buf.to_vec();
        let len = data.len();

        let fut = Box::pin(async move { session.send_chunk(channel_id, data).await });
        self.pending_send = Some(fut);

        // Immediately poll the future once
        if let Some(fut) = self.pending_send.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => {
                    self.pending_send = None;
                    Poll::Ready(Ok(len))
                }
                Poll::Ready(Err(e)) => {
                    self.pending_send = None;
                    Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        format!("send failed: {e:?}"),
                    )))
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Ready(Ok(len))
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        // Wait for pending send to complete
        if let Some(fut) = self.pending_send.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(())) => {
                    self.pending_send = None;
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Err(e)) => {
                    self.pending_send = None;
                    Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        format!("send failed: {e:?}"),
                    )))
                }
                Poll::Pending => Poll::Pending,
            }
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.closed {
            return Poll::Ready(Ok(()));
        }

        // First, flush any pending send
        if self.pending_send.is_some() {
            match self.as_mut().poll_flush(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        self.closed = true;
        let channel_id = self.channel_id;
        let session = self.session.clone();

        // Close the tunnel (fire and forget is OK here since we're shutting down)
        tokio::spawn(async move {
            if let Err(e) = session.close_tunnel(channel_id).await {
                tracing::debug!(channel_id, error = %e, "failed to close tunnel");
            }
        });

        Poll::Ready(Ok(()))
    }
}

/// Bidirectional stream combining StreamReader for reads and TunnelWriter for writes
#[allow(clippy::type_complexity)]
struct TunnelStream {
    reader: StreamReader<
        tokio_stream::adapters::Map<
            ReceiverStream<TunnelChunk>,
            fn(TunnelChunk) -> Result<Bytes, std::io::Error>,
        >,
        Bytes,
    >,
    writer: TunnelWriter,
}

impl tokio::io::AsyncRead for TunnelStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for TunnelStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}
