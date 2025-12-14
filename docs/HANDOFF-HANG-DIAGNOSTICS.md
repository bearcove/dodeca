# Handoff: SHM Transport Hang Diagnostics

## Summary

Added SIGUSR1-triggered diagnostics infrastructure to debug intermittent hangs in the SHM hub transport. The hang occurs after several successful RPC calls (typically ~6) to plugins, particularly `execute_code_samples_plugin`.

## What Was Done

### 1. Diagnostic Infrastructure (rapace, branch: `add-hang-diagnostics`)

Added methods to inspect transport state:

- **`HubAllocator::slot_status()`** - Returns slot counts by state (free/allocated/in_flight) per size class
- **`DescRing::ring_status()`** - Returns `RingStatus { visible_head, tail, capacity, len }`
- **`Doorbell::pending_bytes()`** - Uses `ioctl(FIONREAD)` to check pending signal bytes
- **`HubPeerTransport::doorbell_pending_bytes()`** and `peer()` - Expose for plugin diagnostics
- **`HubHostPeerTransport::doorbell_pending_bytes()`** - Expose for host diagnostics

### 2. SIGUSR1 Handler (dodeca, branch: `memory-mystery`)

- **`dodeca-debug` crate** - New crate with:
  - `install_sigusr1_handler()` - Dumps stack traces on SIGUSR1
  - `register_child_pid()` - Forwards SIGUSR1 to plugin processes
  - `register_diagnostic()` - Register custom diagnostic callbacks

- **Host-side diagnostics** (`plugins.rs`):
  - Dumps allocator slot status
  - Dumps per-peer ring status (recv_ring, send_ring)
  - Dumps doorbell pending bytes for each peer

- **Plugin-side diagnostics** (`dodeca-cell-runtime`):
  - Dumps recv/send ring status
  - Dumps doorbell pending bytes

### 3. Slash Commands (`.claude/commands/`)

Created debugging helpers:
- `/ddc-run` - Run ddc with `systemd-run -p MemoryMax=4G`
- `/sigusr1-dump` - Send SIGUSR1 and interpret output
- `/strace-transport` - Trace doorbell/futex syscalls
- `/perf-profile` - Profile with `perf --call-graph fp`
- `/kill-ddc` - Kill all ddc processes
- `/inspect-shm` - Examine hub SHM file

## How to Use

### Reproduce the hang:
```bash
cd /home/amos/bearcove/picante
systemd-run --user --scope -p MemoryMax=4G /home/amos/bearcove/dodeca/target/debug/ddc build
```

### When hung, send SIGUSR1:
```bash
kill -USR1 <ddc_pid>
```

### Expected diagnostic output:
```
--- Hub Transport Diagnostics ---
HubAllocator slots: N total, N free, N allocated, N in_flight
  class[0] (   1024B): ...
  peer[0] "dodeca-mod-xxx": recv_ring(head=X tail=Y len=Z/256) send_ring(...) doorbell_pending=N
--- End Hub Diagnostics ---
```

## Interpreting Results

| Symptom | Meaning |
|---------|---------|
| `recv_ring len > 0` but plugin `doorbell_pending=0` | Missed wakeup - doorbell signal lost |
| `recv_ring len=0` when host is waiting | Host never enqueued (or wrong peer) |
| `doorbell_pending > 0` on plugin | Plugin isn't draining doorbell |
| Many `in_flight` slots | Messages stuck in transit |
| `visible_head == tail` | Ring empty, no pending work |
| `visible_head > tail` | Ring has data waiting |

## Key Files

### rapace (branch: `add-hang-diagnostics`)
- `crates/rapace-transport-shm/src/hub_alloc.rs` - `slot_status()`, `HubSlotStatus`
- `crates/rapace-transport-shm/src/layout.rs` - `ring_status()`, `RingStatus`
- `crates/rapace-transport-shm/src/doorbell.rs` - `pending_bytes()`
- `crates/rapace-transport-shm/src/hub_transport.rs` - `doorbell_pending_bytes()` on both transports

### dodeca (branch: `memory-mystery`)
- `crates/dodeca-debug/src/lib.rs` - SIGUSR1 handler infrastructure
- `crates/dodeca-cell-runtime/src/lib.rs` - Plugin diagnostic registration
- `crates/dodeca/src/plugins.rs` - Host diagnostic registration

## Verified

- [x] Mutex IS being used correctly around `enqueue()` in all `send_frame` implementations
- [x] Doorbell `try_io` fix is in place (avoids readiness race)
- [x] Ring uses proper Release/Acquire ordering for cross-process visibility

## Next Steps

1. **Capture diagnostic output when hung** - The SIGUSR1 output will show ring state
2. **Check if message is in the ring** - `recv_ring len > 0` means it's there
3. **Check doorbell state** - `doorbell_pending` tells if signal was sent/received
4. **If doorbell shows signal sent but plugin waiting** - Investigate epoll/AsyncFd behavior
5. **Consider strace** - `strace -e sendto,recvfrom,epoll_wait` to see syscalls

## IMPORTANT

**ALWAYS run ddc through systemd-run with memory limits:**
```bash
systemd-run --user --scope -p MemoryMax=4G ./target/debug/ddc ...
```
Otherwise the machine may OOM and die.
