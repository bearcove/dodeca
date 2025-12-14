# Hub Implementation Handoff

## Status: Code Complete, Needs Testing

The SHM hub architecture has been implemented but not yet tested end-to-end with actual plugins.

## What Was Done

### rapace changes (`/home/amos/bearcove/rapace/crates/rapace-transport-shm/src/`)

| File | Description |
|------|-------------|
| `hub_layout.rs` | Data structures: HubHeader (256B), PeerEntry (64B), SizeClassHeader, ExtentHeader, HubSlotMeta |
| `hub_alloc.rs` | Lock-free Treiber stack allocator with 5 size classes (1KB, 16KB, 256KB, 4MB, 16MB) |
| `hub_session.rs` | HubHost (creates/manages hub), HubPeer (plugin opens hub), peer registration |
| `hub_transport.rs` | HubHostPeerTransport (host per-peer), HubPeerTransport (plugin), both implement Transport trait |
| `doorbell.rs` | Socketpair + tokio AsyncFd for cross-process wakeup |
| `lib.rs` | Exports for all hub types |

### dodeca changes

| File | Description |
|------|-------------|
| `crates/dodeca/src/plugins.rs` | Creates single HubHost, passes `--hub-path`, `--peer-id`, `--doorbell-fd` to plugins |
| `crates/dodeca-cell-runtime/src/lib.rs` | Accepts hub args, creates HubPeerTransport |

### Documentation

| File | Description |
|------|-------------|
| `docs/SHM-HUB-ARCHITECTURE.md` | Comprehensive architecture documentation |
| `docs/content/internals/plugins.md` | Updated with hub architecture overview |
| `docs/HANDOFF-SHM-REDESIGN.md` | Marked as implemented |

## What Needs Testing

### 1. Build plugins with new runtime

The plugins in `cells/` need to be rebuilt. They use `dodeca-cell-runtime` which now expects hub arguments:

```bash
# Build a plugin (from cells/ directory)
cd cells/mod-minify && cargo build
```

### 2. Test with dodeca

```bash
# Run with memory limit to avoid OOM
systemd-run --user --scope -p MemoryMax=4G \
  ./target/debug/ddc serve path/to/site
```

Expected behavior:
- Should see "created hub SHM at /tmp/dodeca-hub-{pid}.shm"
- Each plugin should log "Connected to host via hub SHM" with its peer_id
- Plugin communication should work (CSS processing, image optimization, etc.)

### 3. Verify hub file

```bash
ls -la /tmp/dodeca-hub-*.shm
# Should be ~115MB (109MB data + headers)
```

## Potential Issues

### 1. Plugin CLI args changed

Old: `--shm-path=/tmp/dodeca-mod-foo-12345.shm`
New: `--hub-path=/tmp/dodeca-hub-12345.shm --peer-id=1 --doorbell-fd=5`

If plugins don't rebuild, they'll fail with "missing --hub-path".

### 2. FD inheritance

The doorbell socketpair FD must be inherited by the child process. The current implementation:
- Creates socketpair without CLOEXEC
- Passes FD number via `--doorbell-fd=N`
- Plugin wraps it with `Doorbell::from_raw_fd()`

If this doesn't work on some systems, check:
- FD is valid after fork
- FD is not closed by other code before plugin uses it

### 3. Allocator edge cases

The Treiber stack allocator handles:
- Multi-producer (all plugins can allocate)
- ABA problem (tagged pointers with generation)
- Crash cleanup (reclaim slots by owner_peer)

But edge cases to watch:
- Rapid alloc/free under contention
- All slots exhausted in a size class
- Plugin crash during in-flight message

## Quick Verification Steps

```bash
# 1. Run rapace hub tests (should all pass)
cd /home/amos/bearcove/rapace
cargo test -p rapace-transport-shm hub_

# 2. Build dodeca
cd /home/amos/bearcove/dodeca
cargo build -p dodeca

# 3. Build one plugin
cd cells/mod-minify
cargo build

# 4. Run with a simple site
cd /home/amos/bearcove/dodeca
DODECA_PLUGIN_PATH=./cells/mod-minify/target/debug \
systemd-run --user --scope -p MemoryMax=4G \
  ./target/debug/ddc build docs/
```

## Key Types

```rust
// Host side
let hub = HubHost::create(&path, HubConfig::default())?;
let peer_info = hub.add_peer()?;  // Returns PeerInfo { peer_id, doorbell, peer_doorbell_fd }
let transport = HubHostPeerTransport::new(hub.clone(), peer_info.peer_id, peer_info.doorbell);

// Plugin side
let peer = HubPeer::open(&hub_path, peer_id)?;
peer.register();
let doorbell = Doorbell::from_raw_fd(doorbell_fd)?;
let transport = HubPeerTransport::new(Arc::new(peer), doorbell, "plugin-name");
```

## Size Classes

| Class | Size | Count | Total | Use Case |
|-------|------|-------|-------|----------|
| 0 | 1KB | 1024 | 1MB | Small RPC |
| 1 | 16KB | 256 | 4MB | Typical payloads |
| 2 | 256KB | 32 | 8MB | Images, CSS |
| 3 | 4MB | 8 | 32MB | Compressed fonts |
| 4 | 16MB | 4 | 64MB | Decompressed fonts |

Total: ~109MB shared pool vs 272MB (17 × 16MB) with old design.
