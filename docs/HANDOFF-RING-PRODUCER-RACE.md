# Handoff: Ring Producer Race Condition

## Status: Still Hanging After Mutex Fix

The SHM hub transport hangs after ~6 successful RPC calls. Multiple fixes have been attempted but the issue persists.

## Root Cause Hypothesis

**Single-producer ring misuse**: `DescRing::enqueue(&mut local_head, ...)` is designed for single-producer use, but `send_frame()` can be called concurrently by multiple async tasks. This races `local_send_head`, corrupting the ring.

## Fixes Applied (in rapace)

1. **Doorbell try_io fix** (`doorbell.rs:171-174`): Changed `wait()` to use `guard.try_io()` instead of manual `clear_ready()` to avoid the readiness race. Doorbell unit tests pass.

2. **Ring producer mutex** (`hub_transport.rs`): Added `AsyncMutex<u64>` for `local_send_head` at lines 47, 217, 609. However, **the mutex may not be used in the actual enqueue path** - need to verify.

## Fixes Applied (in dodeca)

1. **Code-execution async** (`mods/mod-code-execution/src/impl.rs`): Changed `std::process::Command` to `tokio::process::Command` with `.await` to avoid blocking.

2. **CAS migration** (`crates/dodeca/src/cas.rs:42-45`): Handle old canopydb directory by deleting it if found.

## Current Symptom

```
[HOST] execute_code_samples_plugin: calling RPC...
```
Hangs here. All plugins idle in `epoll_wait`. Host waiting in tokio condvar.

## What Works

- First 5-6 RPC calls succeed
- Doorbell unit tests pass (including stress test)
- Hub transport unit tests pass (but they're in-process, not cross-process)

## What to Check Next

1. **Verify mutex is actually used in enqueue path**: Look at `send_frame()` to ensure it locks `local_send_head` before calling `enqueue()`.

2. **Instrument ring state**: Add logging to show `tail`, `visible_head`, and `local_head` when ring appears full. This will reveal if it's truly full vs corrupted.

3. **Cross-process test**: The unit tests are in-process. Need a test that spawns actual child processes.

## Key Files

| File | Description |
|------|-------------|
| `rapace/crates/rapace-transport-shm/src/hub_transport.rs` | Ring send/recv, mutex added |
| `rapace/crates/rapace-transport-shm/src/doorbell.rs` | Doorbell with try_io fix |
| `rapace/crates/rapace-transport-shm/src/layout.rs` | DescRing::enqueue() - single-producer |
| `dodeca/crates/dodeca/src/plugins.rs` | Plugin spawn, FD passing |
| `dodeca/mods/mod-code-execution/src/impl.rs` | Async Command fix |

## Repro

```bash
# Always use memory limit
rm -rf docs/.dodeca.db docs/.cache
systemd-run --user --scope -p MemoryMax=4G ./target/debug/ddc build

# Send SIGUSR1 to dump stack traces
pkill -SIGUSR1 -f "ddc|dodeca-mod"
```

## Debug Output Location

- `/tmp/ddc_*.log` - build output
- SIGUSR1 dumps backtraces to stderr (captured in log)

## Timing Pattern

The hang always occurs around the 6th `execute_code_samples_plugin` call, specifically on `guide/quick-start.md`. Earlier calls complete in milliseconds, but there's a ~7 second gap before the failing call starts (processing bash scripts?).
