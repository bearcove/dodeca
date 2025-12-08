//! Socket transport for separate-process communication
//!
//! Works over any AsyncRead + AsyncWrite (TCP, Unix sockets, pipes, etc.)

use crate::{decode_frame, encode_frame, Connection, FrameKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

/// Run a connection over a socket
///
/// This spawns two tasks:
/// - One to read frames from the socket and dispatch them
/// - One to write outgoing frames to the socket
///
/// Returns a Connection you can use to send requests.
pub async fn run<R, W>(
    mut reader: R,
    mut writer: W,
) -> std::io::Result<(Connection, mpsc::Receiver<(u64, Vec<u8>)>)>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (conn, mut outgoing_rx) = Connection::new();
    let (incoming_tx, incoming_rx) = mpsc::channel(64);

    let pending = conn.pending().clone();

    // Spawn writer task
    tokio::spawn(async move {
        while let Some(frame) = outgoing_rx.recv().await {
            let encoded = match encode_frame(&frame) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if writer.write_all(&encoded).await.is_err() {
                break;
            }
        }
    });

    // Spawn reader task
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        let mut filled = 0usize;

        loop {
            // Read more data
            let n = match reader.read(&mut buf[filled..]).await {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => break,
            };
            filled += n;

            // Try to parse frames
            let mut consumed = 0;
            while consumed + 4 <= filled {
                let len_bytes = &buf[consumed..consumed + 4];
                let frame_len =
                    u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]])
                        as usize;

                if consumed + 4 + frame_len > filled {
                    // Not enough data yet
                    break;
                }

                let frame_bytes = &buf[consumed + 4..consumed + 4 + frame_len];
                if let Ok(frame) = decode_frame(frame_bytes) {
                    match frame.kind {
                        FrameKind::Response => {
                            // Dispatch to waiting request
                            let mut pending = pending.lock().await;
                            if let Some(tx) = pending.remove(&frame.id) {
                                let _ = tx.send(frame.payload);
                            }
                        }
                        FrameKind::Request | FrameKind::Notification => {
                            // Forward to handler
                            let _ = incoming_tx.send((frame.id, frame.payload)).await;
                        }
                    }
                }

                consumed += 4 + frame_len;
            }

            // Shift remaining data to front
            if consumed > 0 {
                buf.copy_within(consumed..filled, 0);
                filled -= consumed;
            }
        }
    });

    Ok((conn, incoming_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn test_socket_transport() {
        // Create a bidirectional pipe
        let (client_stream, server_stream) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, server_write) = tokio::io::split(server_stream);

        // Set up client side
        let (client_conn, _client_incoming) = run(client_read, client_write).await.unwrap();

        // Set up server side
        let (server_conn, mut server_incoming) = run(server_read, server_write).await.unwrap();

        // Spawn server handler
        let server_conn_clone = server_conn.clone();
        tokio::spawn(async move {
            while let Some((id, payload)) = server_incoming.recv().await {
                // Echo back with prefix
                let mut response = b"echo: ".to_vec();
                response.extend_from_slice(&payload);
                let _ = server_conn_clone.respond(id, response).await;
            }
        });

        // Client sends request
        let response = client_conn.request(b"hello".to_vec()).await.unwrap();
        assert_eq!(response, b"echo: hello");

        // Multiple requests
        let r1 = client_conn.request(b"one".to_vec());
        let r2 = client_conn.request(b"two".to_vec());
        let (resp1, resp2) = tokio::join!(r1, r2);
        assert_eq!(resp1.unwrap(), b"echo: one");
        assert_eq!(resp2.unwrap(), b"echo: two");
    }
}
