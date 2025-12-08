//! Rapace: Async RPC over functions or sockets
//!
//! A minimal async RPC system that works over:
//! - Function pointers (same-process, for .so plugins)
//! - Sockets/pipes (separate process or remote)
//!
//! # Wire Protocol
//!
//! Messages are length-prefixed frames:
//! ```text
//! [length: u32 little-endian][frame: postcard-encoded Frame]
//! ```
//!
//! Each frame has an ID for request/response correlation, allowing
//! multiple concurrent calls in both directions.

pub mod service;
#[cfg(unix)]
pub mod shm;
pub mod socket;

// Re-exports for macro use
pub use facet_postcard;
pub use paste;

use facet::Facet;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

/// A frame on the wire
#[derive(Facet, Debug, Clone)]
pub struct Frame {
    /// Unique ID for request/response correlation
    pub id: u64,
    /// What kind of frame this is
    pub kind: FrameKind,
    /// The payload (another postcard-encoded message)
    pub payload: Vec<u8>,
}

/// The kind of frame
#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    /// A request expecting a response
    Request = 0,
    /// A response to a request
    Response = 1,
    /// A one-way notification (no response expected)
    Notification = 2,
}

/// Encode a frame to bytes (length-prefixed)
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, EncodeError> {
    let frame_bytes = facet_postcard::to_vec(frame).map_err(|_| EncodeError::Serialize)?;
    let len = frame_bytes.len() as u32;
    let mut buf = Vec::with_capacity(4 + frame_bytes.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&frame_bytes);
    Ok(buf)
}

/// Decode a frame from bytes (expects length prefix already stripped)
pub fn decode_frame(bytes: &[u8]) -> Result<Frame, DecodeError> {
    facet_postcard::from_bytes(bytes).map_err(|_| DecodeError::Deserialize)
}

/// Read the length prefix from a buffer, returns (length, bytes_consumed)
pub fn read_length_prefix(buf: &[u8]) -> Option<(u32, usize)> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    Some((len, 4))
}

#[derive(Debug)]
pub enum EncodeError {
    Serialize,
}

#[derive(Debug)]
pub enum DecodeError {
    Deserialize,
    Incomplete,
}

/// Pending requests waiting for responses
type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Vec<u8>>>>>;

/// Inner state of a connection (shared via Arc)
struct ConnectionInner {
    /// Send frames out
    tx: mpsc::Sender<Frame>,
    /// Counter for generating unique request IDs
    next_id: AtomicU64,
    /// Pending requests waiting for responses
    pending: PendingRequests,
}

/// A connection that can send and receive frames
#[derive(Clone)]
pub struct Connection {
    inner: Arc<ConnectionInner>,
}

impl Connection {
    /// Create a new connection with the given sender
    ///
    /// Returns the connection and a receiver that should be used to feed
    /// incoming frames (call `handle_incoming` for each received frame)
    pub fn new() -> (Self, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel(64);
        let inner = ConnectionInner {
            tx,
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let conn = Self {
            inner: Arc::new(inner),
        };
        (conn, rx)
    }

    /// Access pending requests (for transport implementations)
    pub fn pending(&self) -> &PendingRequests {
        &self.inner.pending
    }

    /// Send a request and wait for the response
    pub async fn request(&self, payload: Vec<u8>) -> Result<Vec<u8>, RequestError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);

        // Set up the response channel before sending
        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            pending.insert(id, response_tx);
        }

        // Send the request
        let frame = Frame {
            id,
            kind: FrameKind::Request,
            payload,
        };
        self.inner
            .tx
            .send(frame)
            .await
            .map_err(|_| RequestError::SendFailed)?;

        // Wait for response
        response_rx.await.map_err(|_| RequestError::Cancelled)
    }

    /// Send a notification (fire-and-forget)
    pub async fn notify(&self, payload: Vec<u8>) -> Result<(), RequestError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let frame = Frame {
            id,
            kind: FrameKind::Notification,
            payload,
        };
        self.inner
            .tx
            .send(frame)
            .await
            .map_err(|_| RequestError::SendFailed)
    }

    /// Send a response to a request
    pub async fn respond(&self, request_id: u64, payload: Vec<u8>) -> Result<(), RequestError> {
        let frame = Frame {
            id: request_id,
            kind: FrameKind::Response,
            payload,
        };
        self.inner
            .tx
            .send(frame)
            .await
            .map_err(|_| RequestError::SendFailed)
    }

    /// Handle an incoming frame (call this when you receive a frame)
    ///
    /// Returns Some((id, payload)) if this is a request that needs handling,
    /// None if it was a response (already dispatched to waiter)
    pub async fn handle_incoming(&self, frame: Frame) -> Option<(u64, Vec<u8>)> {
        match frame.kind {
            FrameKind::Response => {
                // Find and notify the waiter
                let mut pending = self.inner.pending.lock().await;
                if let Some(tx) = pending.remove(&frame.id) {
                    let _ = tx.send(frame.payload);
                }
                None
            }
            FrameKind::Request => {
                // Return to caller for handling
                Some((frame.id, frame.payload))
            }
            FrameKind::Notification => {
                // Return to caller for handling (id is ignored for notifications)
                Some((frame.id, frame.payload))
            }
        }
    }
}

#[derive(Debug)]
pub enum RequestError {
    SendFailed,
    Cancelled,
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestError::SendFailed => write!(f, "failed to send request"),
            RequestError::Cancelled => write!(f, "request cancelled"),
        }
    }
}

impl std::error::Error for RequestError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_roundtrip() {
        let frame = Frame {
            id: 42,
            kind: FrameKind::Request,
            payload: b"hello world".to_vec(),
        };

        let encoded = encode_frame(&frame).unwrap();

        // Check length prefix
        let (len, consumed) = read_length_prefix(&encoded).unwrap();
        assert_eq!(consumed, 4);

        // Decode the frame
        let decoded = decode_frame(&encoded[4..4 + len as usize]).unwrap();
        assert_eq!(decoded.id, 42);
        assert_eq!(decoded.kind, FrameKind::Request);
        assert_eq!(decoded.payload, b"hello world");
    }

    #[tokio::test]
    async fn test_request_response() {
        let (conn1, mut rx1) = Connection::new();
        let (conn2, mut rx2) = Connection::new();

        // Spawn handler for conn2's outgoing (which is conn1's incoming)
        let conn1_pending = conn1.pending().clone();
        let handle1 = tokio::spawn(async move {
            while let Some(frame) = rx2.recv().await {
                // This is a frame from conn2, deliver to conn1
                let mut pending = conn1_pending.lock().await;
                if frame.kind == FrameKind::Response {
                    if let Some(tx) = pending.remove(&frame.id) {
                        let _ = tx.send(frame.payload);
                    }
                }
            }
        });

        // Spawn handler for conn1's outgoing (which is conn2's incoming)
        let conn2_clone = conn2.clone();
        let handle2 = tokio::spawn(async move {
            while let Some(frame) = rx1.recv().await {
                // This is a request from conn1, handle it
                if frame.kind == FrameKind::Request {
                    // Echo back with "response: " prefix
                    let mut response = b"response: ".to_vec();
                    response.extend_from_slice(&frame.payload);
                    let _ = conn2_clone.respond(frame.id, response).await;
                }
            }
        });

        // Send a request from conn1
        let response = conn1.request(b"hello".to_vec()).await.unwrap();
        assert_eq!(response, b"response: hello");

        // Clean up
        drop(conn1);
        drop(conn2);
        let _ = handle1.await;
        let _ = handle2.await;
    }
}
