# Consultant Brief: SHM Hub Doorbell Wake Issue

## Problem Summary

Dodeca uses a shared memory (SHM) hub for IPC between a host process (`ddc`) and 16 plugin processes. After several successful RPC calls, the system hangs: the host waits for ring buffer space while plugins sit idle in `epoll_wait`.

## Architecture

- **Host**: `ddc` binary, multi-threaded tokio runtime
- **Plugins**: 16 separate processes, each with `current_thread` tokio runtime
- **Communication**: Single SHM hub file + per-plugin socketpair "doorbell" for wakeup
- **Ring buffers**: Each peer has send/recv rings; when full, sender waits via futex

## The Bug

1. Host sends RPC request, writes to plugin's recv ring
2. Host rings doorbell (writes byte to socketpair)
3. Ring becomes full, host blocks in futex waiting for space
4. Plugin should wake from doorbell, drain ring, signal space available
5. **BUG: Plugin never wakes** - stays in `epoll_wait`

## Evidence from SIGUSR1 Stack Traces

**Host (ddc)**:
```
futex_wait -> parking_lot condvar -> tokio multi_thread block_on
```
Waiting for async work to complete (RPC response).

**Plugins (e.g., mod-fonts, mod-code-execution)**:
```
epoll_wait -> mio select -> tokio current_thread park
```
Idle, no work pending. Doorbell AsyncFd not triggering.

## What Works

- First ~10 RPC calls succeed (extract_css, execute_code_samples x5)
- Plugins connect and register correctly
- Ring buffer allocation works initially

## What We've Already Fixed

1. **Doorbell race condition**: Plugin now checks for pending data before awaiting
2. **Blocking futex_wait**: Moved to `spawn_blocking` to not freeze async runtime
3. **Blocking Command::output**: Changed `std::process::Command` to `tokio::process::Command`

## Suspected Causes

1. **Doorbell FD not registered with epoll correctly after some point**
2. **AsyncFd edge-triggered but byte already consumed, never re-armed**
3. **Host rings wrong peer's doorbell** (unlikely - worked initially)
4. **Ring full before doorbell byte written** - race between ring write and doorbell

## Key Files

| Location | Description |
|----------|-------------|
| `rapace/crates/rapace-transport-shm/src/hub_transport.rs` | Ring buffer send/recv, futex wait |
| `rapace/crates/rapace-transport-shm/src/doorbell.rs` | Socketpair doorbell, AsyncFd wrap |
| `rapace/crates/rapace-transport-shm/src/hub_session.rs` | HubHost::add_peer creates doorbells |
| `dodeca/crates/dodeca-plugin-runtime/src/lib.rs` | Plugin creates HubPeerTransport |
| `dodeca/crates/dodeca/src/plugins.rs` | Host spawns plugins, passes doorbell FDs |

## Reproduction

```bash
# ALWAYS use memory limit to prevent OOM
systemd-run --user --scope -p MemoryMax=4G ./target/debug/ddc build

# Send SIGUSR1 to dump stack traces
pkill -SIGUSR1 -f "ddc|dodeca-mod"
```

## Questions for Investigation

1. Is the doorbell byte being written by host when ring is written?
2. Is AsyncFd using edge-triggered or level-triggered mode?
3. After successful RPC, is the doorbell properly re-armed for next wake?
4. Is there a race where plugin drains ring but misses the doorbell signal?

## Constraints

- Must work under 4G memory limit (cgroup)
- Plugins use `current_thread` runtime - any blocking call freezes everything
- Host uses multi-threaded runtime with ~80 worker threads
