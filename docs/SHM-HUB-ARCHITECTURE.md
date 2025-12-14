# SHM Hub Architecture

## Overview

The SHM Hub architecture replaces the per-plugin SHM files with a single shared memory "hub" file. This design enables variable-size payload allocation and more efficient memory usage across all plugins.

## Problem Statement

The original per-plugin SHM design had limitations:

- **Fixed slot sizes**: Each plugin had fixed 512KB slots
- **Memory waste**: 17 plugins × 16MB = 272MB total, mostly unused
- **Large payload limitation**: Font decompression produces ~10MB but slots were only 512KB
- **No sharing**: Each plugin's memory was isolated, even when one needed more

## Solution: Shared Hub with Size Classes

All plugins now share a single SHM file (~109MB) with variable-size slot allocation via size classes:

| Class | Slot Size | Count | Memory | Purpose |
|-------|-----------|-------|--------|---------|
| 0 | 1KB | 1024 | 1MB | Small RPC args |
| 1 | 16KB | 256 | 4MB | Typical payloads |
| 2 | 256KB | 32 | 8MB | Images, CSS |
| 3 | 4MB | 8 | 32MB | Compressed fonts |
| 4 | 16MB | 4 | 64MB | Decompressed fonts |
| **Total** | | **~1324** | **~109MB** | |

## Memory Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ HUB HEADER (256 bytes)                                          │
│   magic: "RAPAHUB\0", version, max_peers, peer_id_counter       │
│   current_size (atomic), extent_count (atomic)                  │
├─────────────────────────────────────────────────────────────────┤
│ PEER TABLE (max_peers entries, ~64 bytes each)                  │
│   Per peer: peer_id, flags, epoch, last_seen, futex words       │
│   Per peer: send_ring_offset, recv_ring_offset                  │
├─────────────────────────────────────────────────────────────────┤
│ RING REGION (max_peers × 2 rings × ~17KB each)                  │
│   Each ring: DescRingHeader (192B) + 256 × MsgDescHot (64B)     │
├─────────────────────────────────────────────────────────────────┤
│ SIZE CLASS HEADERS (5 × 128 bytes)                              │
│   Per class: slot_size, current_slot_count, free_head           │
├─────────────────────────────────────────────────────────────────┤
│ EXTENT REGION (initial extents, one per size class)             │
│   Extent: [ExtentHeader][SlotMeta×N][SlotData×N]                │
└─────────────────────────────────────────────────────────────────┘
```

## Ring Architecture

Each plugin gets its own ring pair within the shared SHM:

```
Plugin 1 ──[send_ring]──► ┌────────┐ ──[recv_ring_1]──► Plugin 1
Plugin 2 ──[send_ring]──► │  HOST  │ ──[recv_ring_2]──► Plugin 2
Plugin N ──[send_ring]──► │ (mux)  │ ──[recv_ring_N]──► Plugin N
                          └────────┘
```

- **Plugin sends**: Enqueue to its send_ring, signal host via doorbell
- **Host receives**: Uses `HubHostPeerTransport` per plugin
- **Host sends**: Enqueue to specific plugin's recv_ring
- **Plugin receives**: Dequeue from its recv_ring

## Slot Allocation

### Treiber Stack Free Lists

Each size class maintains a lock-free Treiber stack for O(1) allocation:

```rust
struct SizeClassHeader {
    slot_size: u32,
    free_head: AtomicU64,        // (tag<<32) | index, ABA-safe
    slot_available: AtomicU32,   // futex for blocking alloc
    // ...
}
```

### Slot Reference Encoding

Slot references are encoded in `payload_slot` field:
- Bits [31:29]: Size class (0-7)
- Bits [28:0]: Global index within class

```rust
pub fn encode_slot_ref(class: u8, global_index: u32) -> u32 {
    ((class as u32) << 29) | (global_index & 0x1FFF_FFFF)
}

pub fn decode_slot_ref(slot_ref: u32) -> (u8, u32) {
    let class = (slot_ref >> 29) as u8;
    let global_index = slot_ref & 0x1FFF_FFFF;
    (class, global_index)
}
```

### Allocation Algorithm

1. Find smallest class that fits the payload
2. Pop from that class's free list (CAS loop)
3. If empty, try next larger class
4. Set slot state to `Allocated`, bump generation
5. Record owner_peer for crash cleanup

## Cross-Process Wakeup: Socketpair Doorbells

Instead of polling or futex_waitv, each peer has a Unix socketpair:

```
┌────────────────┐                    ┌────────────────┐
│     Host       │                    │    Plugin      │
│                │                    │                │
│  doorbell.fd ◄─┼── socketpair ─────►┼─ doorbell.fd   │
│                │   SOCK_DGRAM       │                │
└────────────────┘                    └────────────────┘
```

- **Bidirectional**: Either side can `send()` to wake the other
- **Non-blocking**: `MSG_DONTWAIT`, ignore `EAGAIN`
- **Async integration**: Wrapped in `tokio::io::unix::AsyncFd`
- **Cross-platform**: Works on both Linux (epoll) and macOS (kqueue)

### Doorbell Protocol

```rust
// Signal the other side (non-blocking)
doorbell.signal();

// Wait for signal (async)
doorbell.wait().await;

// Drain pending signals
doorbell.drain();
```

## Plugin Lifecycle

### Host Side (dodeca)

```rust
// 1. Create hub at startup
let hub = HubHost::create(&hub_path, HubConfig::default())?;

// 2. For each plugin, add a peer
let peer_info = hub.add_peer()?;
// peer_info contains: peer_id, doorbell (host side), peer_doorbell_fd

// 3. Spawn plugin with hub args
cmd.arg(format!("--hub-path={}", hub_path))
   .arg(format!("--peer-id={}", peer_info.peer_id))
   .arg(format!("--doorbell-fd={}", peer_info.peer_doorbell_fd));

// 4. Close the peer's doorbell fd (plugin inherits it)
close_peer_fd(peer_info.peer_doorbell_fd);

// 5. Create per-peer transport
let transport = HubHostPeerTransport::new(hub.clone(), peer_id, peer_info.doorbell);
let rpc_session = RpcSession::with_channel_start(transport, 1);
```

### Plugin Side (dodeca-mod-*)

```rust
// 1. Parse args
let args = parse_args()?;  // --hub-path, --peer-id, --doorbell-fd

// 2. Open hub as peer
let peer = HubPeer::open(&args.hub_path, args.peer_id)?;
peer.register();

// 3. Create doorbell from inherited fd
let doorbell = Doorbell::from_raw_fd(args.doorbell_fd)?;

// 4. Create transport
let transport = HubPeerTransport::new(Arc::new(peer), doorbell, plugin_name);
let rpc_session = RpcSession::with_channel_start(transport, 2);
```

## Crash Cleanup

When a plugin dies, the host:

1. **Drains dead peer's send ring**: Drop any descriptors still queued
2. **Reclaims slots**: Scan slots where `owner_peer == dead_peer_id`
   - Force state to `Free`
   - Bump generation (invalidates stale descriptors)
   - Clear owner
3. **Marks peer entry inactive**: `flags.store(DEAD)`

```rust
// In the child wait task
tokio::task::spawn_blocking(move || {
    child.wait().ok();
    hub.allocator().reclaim_peer_slots(peer_id as u32);
});
```

## Files

### rapace (new)

| File | Purpose |
|------|---------|
| `hub_layout.rs` | HubHeader, PeerEntry, SizeClassHeader, ExtentHeader, SlotMeta |
| `hub_alloc.rs` | Per-class Treiber stack allocator |
| `hub_session.rs` | HubHost (creates hub), HubPeer (opens hub) |
| `hub_transport.rs` | HubHostPeerTransport (host), HubPeerTransport (plugin) |
| `doorbell.rs` | Socketpair + AsyncFd wrapper |

### dodeca (modified)

| File | Changes |
|------|---------|
| `plugins.rs` | Single HubHost, pass --hub-path/--peer-id/--doorbell-fd |
| `plugin-runtime/lib.rs` | Accept hub args, create HubPeerTransport |

## Transport Types

| Type | Side | Implements | Purpose |
|------|------|------------|---------|
| `HubHostPeerTransport` | Host | `Transport` | Per-peer adapter for RpcSession |
| `HubPeerTransport` | Plugin | `Transport` | Plugin-side transport |
| `HubHostTransport` | Host | - | Multi-peer demux (for custom use) |

## Configuration

Default hub configuration:

```rust
pub const HUB_SIZE_CLASSES: &[(u32, u32)] = &[
    (1024, 1024),       // 1KB × 1024 = 1MB
    (16384, 256),       // 16KB × 256 = 4MB
    (262144, 32),       // 256KB × 32 = 8MB
    (4194304, 8),       // 4MB × 8 = 32MB
    (16777216, 4),      // 16MB × 4 = 64MB
];

pub const DEFAULT_MAX_PEERS: u16 = 32;
pub const DEFAULT_RING_CAPACITY: u32 = 256;
```

## Benefits

1. **Variable-size payloads**: Font decompression can use 16MB slots
2. **Memory efficiency**: Shared pool instead of per-plugin allocation
3. **O(1) allocation**: Lock-free Treiber stacks
4. **No polling**: Socketpair doorbells with async I/O
5. **Crash safety**: Generation counters invalidate stale references
6. **Cross-platform**: Linux epoll, macOS kqueue via tokio AsyncFd

## Future: Dynamic Growth

The architecture supports appending new extents:

1. Host: `ftruncate(fd, new_size)`
2. Host: `mremap()` or remap to extend mapping
3. Host: Initialize new extent, link into class list
4. Host: `current_size.store(new_size, Release)`
5. Plugins: Detect via `current_size` change, remap
