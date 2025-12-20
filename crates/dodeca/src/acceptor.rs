#![allow(clippy::collapsible_if)]

use async_send_fd::{AsyncRecvFd, AsyncSendFd};
use eyre::{Result, eyre};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, UnixStream};
use tokio::sync::mpsc;
use tracing_subscriber::prelude::*;

fn init_tracing() {
    let filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::filter::EnvFilter::new("info"));

    let fmt_layer = tracing_subscriber::fmt::layer().with_target(true).compact();
    tracing_subscriber::registry()
        .with(
            fmt_layer
                .with_timer(tracing_subscriber::fmt::time::SystemTime)
                .with_filter(filter),
        )
        .init();
}

#[derive(Debug)]
struct AcceptorArgs {
    fd_socket: String,
    acceptor_socket: String,
    queue_size: usize,
}

fn parse_args() -> Result<AcceptorArgs> {
    let mut fd_socket = None;
    let mut acceptor_socket = None;
    let mut queue_size = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fd-socket" => {
                fd_socket = args.next();
            }
            "--acceptor-socket" => {
                acceptor_socket = args.next();
            }
            "--queue-size" => {
                queue_size = args.next();
            }
            "--help" | "-h" => {
                eprintln!(
                    "Usage: ddc-acceptor --fd-socket <path> --acceptor-socket <path> [--queue-size <n>]"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }

    let fd_socket = fd_socket.ok_or_else(|| eyre!("--fd-socket is required"))?;
    let acceptor_socket = acceptor_socket.ok_or_else(|| eyre!("--acceptor-socket is required"))?;
    let queue_size = queue_size
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(128);

    Ok(AcceptorArgs {
        fd_socket,
        acceptor_socket,
        queue_size,
    })
}

async fn receive_listener(fd_socket: &str) -> Result<TcpListener> {
    tracing::info!(%fd_socket, "Connecting to Unix socket for FD passing");
    let unix_stream = UnixStream::connect(fd_socket)
        .await
        .map_err(|e| eyre!("Failed to connect to fd-socket {}: {}", fd_socket, e))?;

    tracing::info!(%fd_socket, "Receiving TCP listener FD from harness");
    let fd = unix_stream
        .recv_fd()
        .await
        .map_err(|e| eyre!("Failed to receive listener FD: {}", e))?;

    // SAFETY: The harness created a valid TcpListener and sent its FD.
    let std_listener = unsafe { std::net::TcpListener::from_raw_fd(fd) };
    std_listener
        .set_nonblocking(true)
        .map_err(|e| eyre!("Failed to set listener to non-blocking: {}", e))?;

    let listener = TcpListener::from_std(std_listener)
        .map_err(|e| eyre!("Failed to convert listener to tokio: {}", e))?;
    tracing::info!(%fd_socket, "Successfully received TCP listener FD");

    Ok(listener)
}

async fn connect_with_retry(path: &str) -> UnixStream {
    let mut backoff = Duration::from_millis(25);
    loop {
        match UnixStream::connect(path).await {
            Ok(stream) => {
                tracing::info!(%path, "Connected to host acceptor socket");
                return stream;
            }
            Err(e) => {
                tracing::warn!(%path, error = %e, "Host socket not ready, retrying");
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(1));
            }
        }
    }
}

async fn wait_for_host_socket(path: &str) {
    let stream = connect_with_retry(path).await;
    drop(stream);
}

async fn send_loop(
    acceptor_socket: String,
    mut rx: mpsc::Receiver<OwnedFd>,
    queue_depth: Arc<AtomicUsize>,
) -> Result<()> {
    let mut pending: Option<OwnedFd> = None;
    loop {
        let mut stream = connect_with_retry(&acceptor_socket).await;

        loop {
            let fd = if let Some(fd) = pending.take() {
                fd
            } else {
                match rx.recv().await {
                    Some(fd) => fd,
                    None => return Ok(()),
                }
            };

            if let Err(e) = stream.send_fd(fd.as_raw_fd()).await {
                tracing::warn!(error = %e, "Failed to send FD to host, reconnecting");
                pending = Some(fd);
                break;
            }

            let mut ack = [0u8; 1];
            match tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut ack)).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "Failed to read ack from host");
                    break;
                }
                Err(_) => {
                    tracing::warn!("Timed out waiting for host ack");
                }
            }

            let prev = queue_depth.fetch_sub(1, Ordering::Relaxed);
            let depth = prev.saturating_sub(1);
            tracing::debug!(queue_depth = depth, "Sent accepted connection FD to host");
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    tx: mpsc::Sender<OwnedFd>,
    queue_depth: Arc<AtomicUsize>,
) -> Result<()> {
    tracing::info!("Accept loop starting");
    let mut accept_seq: u64 = 0;
    loop {
        let (stream, addr) = listener.accept().await?;
        let conn_id = accept_seq;
        accept_seq = accept_seq.wrapping_add(1);
        tracing::info!(conn_id, ?addr, "Accepted browser connection (acceptor)");

        let std_stream = stream
            .into_std()
            .map_err(|e| eyre!("Failed to convert stream to std: {}", e))?;
        let fd = std_stream.into_raw_fd();
        // SAFETY: raw fd is owned by this function now.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };

        if tx.send(owned).await.is_err() {
            tracing::warn!("Host receiver gone, dropping accepted connection");
            break;
        }
        let depth = queue_depth.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::debug!(queue_depth = depth, "Queued accepted connection FD");
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    init_tracing();
    let args = parse_args()?;

    tracing::info!(
        fd_socket = %args.fd_socket,
        acceptor_socket = %args.acceptor_socket,
        queue_size = args.queue_size,
        "Starting acceptor"
    );

    let listener = receive_listener(&args.fd_socket).await?;
    wait_for_host_socket(&args.acceptor_socket).await;

    let (tx, rx) = mpsc::channel::<OwnedFd>(args.queue_size);
    let queue_depth = Arc::new(AtomicUsize::new(0));
    let acceptor_socket = args.acceptor_socket.clone();

    let queue_depth_accept = queue_depth.clone();
    let accept_task =
        tokio::spawn(async move { accept_loop(listener, tx, queue_depth_accept).await });
    let send_task = tokio::spawn(async move { send_loop(acceptor_socket, rx, queue_depth).await });

    let accept_result = accept_task.await?;
    let send_result = send_task.await?;

    accept_result?;
    send_result?;

    Ok(())
}
