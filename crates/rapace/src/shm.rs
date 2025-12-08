//! Shared memory ring buffer transport
//!
//! A lock-free, single-producer single-consumer ring buffer in shared memory.
//! Used for fast local communication between host and plugin.
//!
//! # Memory Layout
//!
//! ```text
//! [Ring A: host → plugin][Ring B: plugin → host]
//!
//! Each ring:
//! ┌─────────────────────────────────────────────────────────┐
//! │ head: AtomicU64 │ tail: AtomicU64 │ data: [u8; capacity] │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! - `head`: write position (updated by producer)
//! - `tail`: read position (updated by consumer)
//! - Ring is empty when head == tail
//! - Ring is full when (head + 1) % capacity == tail
//!
//! Messages are length-prefixed: [len: u32][data: [u8; len]]

use std::sync::atomic::{AtomicU64, Ordering};
use std::{io, ptr};

/// Size of ring buffer header (head + tail pointers)
const RING_HEADER_SIZE: usize = 16; // 2 * size_of::<AtomicU64>()

/// Default ring buffer capacity (data portion)
pub const DEFAULT_RING_CAPACITY: usize = 64 * 1024; // 64 KB

/// Total size needed for a ring buffer
pub const fn ring_total_size(capacity: usize) -> usize {
    RING_HEADER_SIZE + capacity
}

/// Total size needed for the shared memory region (two rings)
pub const fn shm_total_size(ring_capacity: usize) -> usize {
    2 * ring_total_size(ring_capacity)
}

/// A ring buffer in shared memory
pub struct Ring {
    /// Pointer to the start of this ring's memory
    base: *mut u8,
    /// Capacity of the data portion
    capacity: usize,
}

// Safety: Ring uses atomic operations for synchronization
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

impl Ring {
    /// Create a Ring from a pointer to its memory region
    ///
    /// # Safety
    /// - `base` must point to a valid memory region of at least `ring_total_size(capacity)` bytes
    /// - The memory must be properly aligned for AtomicU64
    /// - The memory must remain valid for the lifetime of this Ring
    pub unsafe fn from_ptr(base: *mut u8, capacity: usize) -> Self {
        Self { base, capacity }
    }

    /// Initialize the ring (call once when creating shared memory)
    pub fn init(&self) {
        self.head().store(0, Ordering::Release);
        self.tail().store(0, Ordering::Release);
    }

    fn head(&self) -> &AtomicU64 {
        unsafe { &*(self.base as *const AtomicU64) }
    }

    fn tail(&self) -> &AtomicU64 {
        unsafe { &*((self.base as *const AtomicU64).add(1)) }
    }

    fn data(&self) -> *mut u8 {
        unsafe { self.base.add(RING_HEADER_SIZE) }
    }

    /// Available space for writing
    pub fn write_available(&self) -> usize {
        let head = self.head().load(Ordering::Acquire);
        let tail = self.tail().load(Ordering::Acquire);
        if head >= tail {
            self.capacity - (head - tail) as usize - 1
        } else {
            (tail - head) as usize - 1
        }
    }

    /// Available data for reading
    pub fn read_available(&self) -> usize {
        let head = self.head().load(Ordering::Acquire);
        let tail = self.tail().load(Ordering::Acquire);
        if head >= tail {
            (head - tail) as usize
        } else {
            self.capacity - (tail - head) as usize
        }
    }

    /// Write a message to the ring (length-prefixed)
    ///
    /// Returns Ok(()) if written, Err if not enough space
    pub fn write_message(&self, data: &[u8]) -> Result<(), WriteError> {
        let msg_len = data.len();
        let total_len = 4 + msg_len; // length prefix + data

        if total_len > self.write_available() {
            return Err(WriteError::Full);
        }

        let head = self.head().load(Ordering::Acquire) as usize;
        let capacity = self.capacity;

        // Write length prefix
        let len_bytes = (msg_len as u32).to_le_bytes();
        self.write_bytes_at(head, &len_bytes, capacity);

        // Write data
        self.write_bytes_at((head + 4) % capacity, data, capacity);

        // Update head
        let new_head = (head + total_len) % capacity;
        self.head().store(new_head as u64, Ordering::Release);

        Ok(())
    }

    fn write_bytes_at(&self, pos: usize, data: &[u8], capacity: usize) {
        let data_ptr = self.data();
        let first_chunk = (capacity - pos).min(data.len());

        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), data_ptr.add(pos), first_chunk);
            if first_chunk < data.len() {
                // Wrap around
                ptr::copy_nonoverlapping(
                    data.as_ptr().add(first_chunk),
                    data_ptr,
                    data.len() - first_chunk,
                );
            }
        }
    }

    /// Read a message from the ring
    ///
    /// Returns Ok(Some(data)) if a message was read, Ok(None) if ring is empty
    pub fn read_message(&self) -> Result<Option<Vec<u8>>, ReadError> {
        if self.read_available() < 4 {
            return Ok(None);
        }

        let tail = self.tail().load(Ordering::Acquire) as usize;
        let capacity = self.capacity;

        // Read length prefix
        let mut len_bytes = [0u8; 4];
        self.read_bytes_at(tail, &mut len_bytes, capacity);
        let msg_len = u32::from_le_bytes(len_bytes) as usize;

        let total_len = 4 + msg_len;
        if self.read_available() < total_len {
            // Incomplete message (shouldn't happen with proper writes)
            return Err(ReadError::Incomplete);
        }

        // Read data
        let mut data = vec![0u8; msg_len];
        self.read_bytes_at((tail + 4) % capacity, &mut data, capacity);

        // Update tail
        let new_tail = (tail + total_len) % capacity;
        self.tail().store(new_tail as u64, Ordering::Release);

        Ok(Some(data))
    }

    fn read_bytes_at(&self, pos: usize, buf: &mut [u8], capacity: usize) {
        let data_ptr = self.data();
        let first_chunk = (capacity - pos).min(buf.len());

        unsafe {
            ptr::copy_nonoverlapping(data_ptr.add(pos), buf.as_mut_ptr(), first_chunk);
            if first_chunk < buf.len() {
                // Wrap around
                ptr::copy_nonoverlapping(
                    data_ptr,
                    buf.as_mut_ptr().add(first_chunk),
                    buf.len() - first_chunk,
                );
            }
        }
    }
}

#[derive(Debug)]
pub enum WriteError {
    Full,
}

#[derive(Debug)]
pub enum ReadError {
    Incomplete,
}

/// Shared memory channel - owns the memory and provides both rings
pub struct SharedMemoryChannel {
    /// The shared memory region
    mem: SharedMemory,
    /// Capacity of each ring's data portion
    ring_capacity: usize,
}

impl SharedMemoryChannel {
    /// Create a new shared memory channel
    pub fn new(ring_capacity: usize) -> io::Result<Self> {
        let total_size = shm_total_size(ring_capacity);
        let mem = SharedMemory::create(total_size)?;

        let channel = Self { mem, ring_capacity };

        // Initialize both rings
        channel.ring_a().init();
        channel.ring_b().init();

        Ok(channel)
    }

    /// Open an existing shared memory channel by name/fd
    pub fn open(name: &str, ring_capacity: usize) -> io::Result<Self> {
        let total_size = shm_total_size(ring_capacity);
        let mem = SharedMemory::open(name, total_size)?;
        Ok(Self { mem, ring_capacity })
    }

    /// Get the name/path for sharing with another process
    pub fn name(&self) -> &str {
        self.mem.name()
    }

    /// Ring A: typically host → plugin
    pub fn ring_a(&self) -> Ring {
        unsafe { Ring::from_ptr(self.mem.ptr(), self.ring_capacity) }
    }

    /// Ring B: typically plugin → host
    pub fn ring_b(&self) -> Ring {
        let offset = ring_total_size(self.ring_capacity);
        unsafe { Ring::from_ptr(self.mem.ptr().add(offset), self.ring_capacity) }
    }
}

/// Platform-specific shared memory implementation
struct SharedMemory {
    ptr: *mut u8,
    size: usize,
    name: String,
    #[cfg(unix)]
    fd: std::os::unix::io::RawFd,
}

impl SharedMemory {
    #[cfg(unix)]
    fn create(size: usize) -> io::Result<Self> {
        use std::ffi::CString;
        use std::os::unix::io::RawFd;

        // Generate unique name
        let name = format!("/rapace-{}-{}", std::process::id(), rand_u64());
        let c_name = CString::new(name.clone()).unwrap();

        unsafe {
            // Create shared memory object
            let fd: RawFd = libc::shm_open(
                c_name.as_ptr(),
                libc::O_CREAT | libc::O_RDWR | libc::O_EXCL,
                0o600,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }

            // Set size
            if libc::ftruncate(fd, size as libc::off_t) < 0 {
                libc::close(fd);
                libc::shm_unlink(c_name.as_ptr());
                return Err(io::Error::last_os_error());
            }

            // Map it
            let ptr = libc::mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                libc::close(fd);
                libc::shm_unlink(c_name.as_ptr());
                return Err(io::Error::last_os_error());
            }

            Ok(Self {
                ptr: ptr as *mut u8,
                size,
                name,
                fd,
            })
        }
    }

    #[cfg(unix)]
    fn open(name: &str, size: usize) -> io::Result<Self> {
        use std::ffi::CString;
        use std::os::unix::io::RawFd;

        let c_name = CString::new(name).unwrap();

        unsafe {
            let fd: RawFd = libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }

            let ptr = libc::mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                libc::close(fd);
                return Err(io::Error::last_os_error());
            }

            Ok(Self {
                ptr: ptr as *mut u8,
                size,
                name: name.to_string(),
                fd,
            })
        }
    }

    fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(unix)]
impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.size);
            libc::close(self.fd);
            // Only unlink if we created it (has our PID in the name)
            let our_prefix = format!("/rapace-{}-", std::process::id());
            if self.name.starts_with(&our_prefix) {
                let c_name = std::ffi::CString::new(self.name.clone()).unwrap();
                libc::shm_unlink(c_name.as_ptr());
            }
        }
    }
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    duration.as_nanos() as u64 ^ (duration.as_secs() << 32)
}

// =============================================================================
// Async transport over shared memory
// =============================================================================

use crate::{Connection, FrameKind, decode_frame, encode_frame};
use tokio::sync::mpsc;
use std::time::Duration;

/// Run a connection over shared memory rings
///
/// - `outgoing_ring`: Ring to write outgoing messages to
/// - `incoming_ring`: Ring to read incoming messages from
///
/// Returns a Connection and a receiver for incoming requests/notifications.
pub async fn run(
    outgoing_ring: Ring,
    incoming_ring: Ring,
) -> (Connection, mpsc::Receiver<(u64, Vec<u8>)>) {
    let (conn, mut conn_outgoing) = Connection::new();
    let (incoming_tx, incoming_rx) = mpsc::channel(64);

    let pending = conn.pending().clone();

    // Spawn writer task - polls the outgoing channel and writes to ring
    tokio::spawn(async move {
        while let Some(frame) = conn_outgoing.recv().await {
            let encoded = match encode_frame(&frame) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Spin until we can write (or use a more sophisticated backpressure)
            loop {
                match outgoing_ring.write_message(&encoded) {
                    Ok(()) => break,
                    Err(WriteError::Full) => {
                        // Ring full, yield and retry
                        tokio::task::yield_now().await;
                    }
                }
            }
        }
    });

    // Spawn reader task - polls the incoming ring and dispatches
    tokio::spawn(async move {
        loop {
            match incoming_ring.read_message() {
                Ok(Some(msg)) => {
                    // Skip the length prefix that encode_frame added
                    // (ring already stripped its own length prefix)
                    if msg.len() < 4 {
                        continue;
                    }
                    let frame_data = &msg[4..];
                    if let Ok(frame) = decode_frame(frame_data) {
                        match frame.kind {
                            FrameKind::Response => {
                                let mut pending = pending.lock().await;
                                if let Some(tx) = pending.remove(&frame.id) {
                                    let _ = tx.send(frame.payload);
                                }
                            }
                            FrameKind::Request | FrameKind::Notification => {
                                let _ = incoming_tx.send((frame.id, frame.payload)).await;
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No message available, poll again after short sleep
                    tokio::time::sleep(Duration::from_micros(10)).await;
                }
                Err(_) => {
                    // Error reading, sleep and retry
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
    });

    (conn, incoming_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_basic() {
        let channel = SharedMemoryChannel::new(1024).unwrap();
        let ring = channel.ring_a();

        // Write a message
        ring.write_message(b"hello").unwrap();
        assert_eq!(ring.read_available(), 9); // 4 (len) + 5 (data)

        // Read it back
        let msg = ring.read_message().unwrap().unwrap();
        assert_eq!(msg, b"hello");

        // Ring should be empty
        assert!(ring.read_message().unwrap().is_none());
    }

    #[test]
    fn test_ring_buffer_multiple_messages() {
        let channel = SharedMemoryChannel::new(1024).unwrap();
        let ring = channel.ring_a();

        // Write multiple messages
        ring.write_message(b"one").unwrap();
        ring.write_message(b"two").unwrap();
        ring.write_message(b"three").unwrap();

        // Read them back in order
        assert_eq!(ring.read_message().unwrap().unwrap(), b"one");
        assert_eq!(ring.read_message().unwrap().unwrap(), b"two");
        assert_eq!(ring.read_message().unwrap().unwrap(), b"three");
        assert!(ring.read_message().unwrap().is_none());
    }

    #[test]
    fn test_ring_buffer_wrap_around() {
        let channel = SharedMemoryChannel::new(64).unwrap(); // Small buffer to force wrap
        let ring = channel.ring_a();

        // Fill and drain several times to test wrap-around
        for i in 0..10 {
            let msg = format!("message-{}", i);
            ring.write_message(msg.as_bytes()).unwrap();
            let read = ring.read_message().unwrap().unwrap();
            assert_eq!(read, msg.as_bytes());
        }
    }

    #[test]
    fn test_bidirectional() {
        let channel = SharedMemoryChannel::new(1024).unwrap();

        // Host writes to ring_a, plugin reads from ring_a
        // Plugin writes to ring_b, host reads from ring_b
        let host_to_plugin = channel.ring_a();
        let plugin_to_host = channel.ring_b();

        host_to_plugin.write_message(b"request").unwrap();
        assert_eq!(
            host_to_plugin.read_message().unwrap().unwrap(),
            b"request"
        );

        plugin_to_host.write_message(b"response").unwrap();
        assert_eq!(
            plugin_to_host.read_message().unwrap().unwrap(),
            b"response"
        );
    }

    // =========================================================================
    // NOTE: A bidirectional shm test with services requires two separate processes
    // because Ring is SPSC (single producer, single consumer). In a single process,
    // multiple readers would race on the same ring.
    //
    // See examples/shm_host.rs and examples/shm_plugin.rs for the proper cross-process test.
}
