//! Devtools WebSocket handler.

use std::io;
use std::sync::Arc;

use axum::extract::{
    State, WebSocketUpgrade,
    ws::{Message, WebSocket},
};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use vox::{Backing, Link, LinkRx, LinkTx};

use crate::RouterContext;

/// WebSocket handler for browser devtools.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(ctx): State<Arc<dyn RouterContext>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket: WebSocket| async move {
        tracing::debug!("devtools websocket upgraded");
        let link = AxumWsLink::new(socket);
        let result = vox::acceptor_on(link)
            .on_connection(DevtoolsAcceptor { ctx })
            .establish::<vox::NoopClient>()
            .await;

        match result {
            Ok(root) => {
                root.caller.closed().await;
            }
            Err(error) => {
                tracing::warn!(?error, "devtools websocket vox session failed");
            }
        }
    })
}

#[derive(Clone)]
struct DevtoolsAcceptor {
    ctx: Arc<dyn RouterContext>,
}

impl vox::ConnectionAcceptor for DevtoolsAcceptor {
    fn accept(
        &self,
        request: &vox::ConnectionRequest,
        connection: vox::PendingConnection,
    ) -> Result<(), vox::Metadata> {
        self.ctx
            .accept_devtools_connection(request.service(), connection)
    }
}

struct AxumWsLink {
    socket: WebSocket,
}

impl AxumWsLink {
    fn new(socket: WebSocket) -> Self {
        Self { socket }
    }
}

impl Link for AxumWsLink {
    type Tx = AxumWsLinkTx;
    type Rx = AxumWsLinkRx;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (mut ws_tx, mut ws_rx) = self.socket.split();
        let (tx_out, mut rx_out) = mpsc::channel::<Vec<u8>>(1);
        let (tx_in, rx_in) = mpsc::channel::<Result<Message, io::Error>>(1);

        let io_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    outgoing = rx_out.recv() => {
                        let Some(bytes) = outgoing else {
                            break;
                        };
                        if let Err(error) = ws_tx.send(Message::Binary(bytes.into())).await {
                            let _ = tx_in.send(Err(io::Error::other(error.to_string()))).await;
                            break;
                        }
                    }
                    incoming = ws_rx.next() => {
                        match incoming {
                            Some(Ok(message)) => {
                                let is_close = matches!(message, Message::Close(_));
                                if tx_in.send(Ok(message)).await.is_err() || is_close {
                                    break;
                                }
                            }
                            Some(Err(error)) => {
                                let _ = tx_in.send(Err(io::Error::other(error.to_string()))).await;
                                break;
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        (
            AxumWsLinkTx {
                tx: tx_out,
                io_task,
            },
            AxumWsLinkRx { rx: rx_in },
        )
    }
}

struct AxumWsLinkTx {
    tx: mpsc::Sender<Vec<u8>>,
    io_task: JoinHandle<()>,
}

impl LinkTx for AxumWsLinkTx {
    async fn send(&self, bytes: Vec<u8>) -> io::Result<()> {
        let permit = self.tx.clone().reserve_owned().await.map_err(|_| {
            io::Error::new(io::ErrorKind::ConnectionReset, "websocket task stopped")
        })?;
        drop(permit.send(bytes));
        Ok(())
    }

    async fn close(self) -> io::Result<()> {
        drop(self.tx);
        self.io_task.await.map_err(io::Error::other)
    }
}

struct AxumWsLinkRx {
    rx: mpsc::Receiver<Result<Message, io::Error>>,
}

#[derive(Debug)]
struct AxumWsLinkRxError(io::Error);

impl std::fmt::Display for AxumWsLinkRxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "axum websocket rx: {}", self.0)
    }
}

impl std::error::Error for AxumWsLinkRxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl LinkRx for AxumWsLinkRx {
    type Error = AxumWsLinkRxError;

    async fn recv(&mut self) -> Result<Option<Backing>, Self::Error> {
        loop {
            match self.rx.recv().await {
                Some(Ok(Message::Binary(data))) => {
                    return Ok(Some(Backing::Boxed(Vec::from(data).into_boxed_slice())));
                }
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(Message::Text(_))) => {
                    return Err(AxumWsLinkRxError(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "text frames not allowed on vox websocket link",
                    )));
                }
                Some(Err(error)) => return Err(AxumWsLinkRxError(error)),
            }
        }
    }
}
