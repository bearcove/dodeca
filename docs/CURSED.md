# Cursed: First Test Fails After Build

## Workaround

```bash
cargo xtask integration -- --no-build
```

Run this after an initial `cargo build --release`. Tests pass 100% without rebuilding.

---

## The Problem

The first integration test (`basic::all_pages_return_200`) consistently fails when run immediately after `cargo build`, but passes when run without rebuilding.

```
# This fails:
cargo build --release -p dodeca && target/release/integration-tests basic::all_pages_return_200

# This passes:
target/release/integration-tests basic::all_pages_return_200
```

## Error

```
Server never became HTTP-ready at http://127.0.0.1:XXXXX/__probe__ after 186 attempts:
reqwest::Error { kind: Request, ..., source: hyper::Error(Io, Os { code: 54,
kind: ConnectionReset, message: "Connection reset by peer" }) }
```

The test harness:
1. Binds a TCP socket
2. Spawns ddc with `--fd-socket` (FD passing via Unix socket)
3. Sends the TCP listener FD to ddc
4. Probes `/__probe__` endpoint

The probe connects successfully (TCP handshake completes) but then gets RST.

## What We Ruled Out

### Not timing/flakiness
- Without build: passes 100% of the time
- With build: fails 100% of the time
- This is deterministic, not a race condition

### Not file modification time
```bash
touch target/release/ddc  # Still passes after this
```

### Not a fresh file
```bash
cp target/release/ddc target/release/ddc.new
mv target/release/ddc.new target/release/ddc  # Still passes after this
```

### Not a delay issue
```bash
cargo build && sleep 2 && ./test  # Still fails
```

### Not the binary itself being broken
```bash
cargo build && target/release/ddc serve fixtures/sample-site --no-tui
# Works fine! Serves HTTP correctly.
```

### Not subsequent tests
- Only the FIRST test after build fails
- All other tests pass (even though they also spawn fresh ddc processes)

## What We Know

1. `cargo build` does *something* that breaks the first test
2. Running ddc directly (not via test harness) works fine after build
3. The issue is specific to the FD-passing mechanism (`--fd-socket`)
4. TCP connections succeed but get RST after connecting
5. Subsequent test runs (without rebuild) pass consistently
6. `cargo xtask integration -- --no-build` passes consistently

## Theories

### 1. Cargo holds some resource
Maybe cargo keeps file descriptors or shared memory segments open briefly after build completes, interfering with the test's Unix socket FD passing.

### 2. macOS code signing / dylib cache
First execution of a newly-built binary might trigger some macOS security check or dylib cache rebuild that interferes with the FD passing.

### 3. Something about the build process vs the binary
The issue correlates with `cargo build` running, not with the binary changing. Something cargo does (not the resulting binary) causes this.

### 4. Process group / session weirdness
The test harness uses `ur_taking_me_with_you::spawn_dying_with_parent()` for death-watch. Maybe there's interaction with cargo's process tree.

## macOS Console Logs

FSEvents errors appear but happen on ALL ddc processes (not just failing ones):
```
FSEventsPurgeEventsForDeviceUpToEventId: f2d_purge_events_for_device_up_to_event_id_rpc() failed: 5
```

These are probably unrelated - they appear during passing tests too.

## Root Cause Found: HTTP Cell Immediately Closes Tunnel

With debug logging, we found the exact failure:

```
RPC call start service="TcpTunnel" method="open" channel_id=357
received frame channel_id=357 ... payload_len=2           # RPC response (TunnelHandle)
RPC call complete
received frame channel_id=358 ... DATA | EOS payload_len=0  # Tunnel immediately gets EOS!
try_route_to_tunnel: EOS received, removing tunnel
browser <-> tunnel finished to_tunnel=0 to_browser=0
```

The HTTP cell successfully opens the tunnel (channel N), but then **immediately sends EOS on channel N+1** (the tunnel stream) with 0 bytes. The cell closes the tunnel before any HTTP data can flow.

This results in `copy_bidirectional` returning `(0, 0)` - zero bytes transferred in both directions.

## Environment

- macOS 15.5 (Darwin 25.1.0) on Apple M4 Pro
- rustc 1.91.1 (ed61e7d7e 2025-11-07)
- cargo 1.91.1
- Release build only (debug not tested)

## Minimal Repro Script

```bash
#!/bin/bash
set -e
echo "Building..."
cargo build --release -p dodeca
echo "Running test (should fail)..."
DODECA_BIN=target/release/ddc DODECA_CELL_PATH=target/release \
  target/release/integration-tests basic::all_pages_return_200
```

## Key Files

- **FD passing (harness):** `crates/integration-tests/src/harness.rs:120-227`
- **FD receiving (host):** `crates/dodeca/src/main.rs:1694-1720`
- **Accept loop:** `crates/dodeca/src/cell_server.rs:266-311`
- **Tunnel bridging:** `crates/dodeca/src/cell_server.rs:331-365`
- **HTTP cell tunnel impl:** `cells/cell-http/src/tunnel.rs`

## Next Steps

1. **Instrument HTTP cell** - Add logging to `cells/cell-http/src/tunnel.rs` to see why the cell immediately closes the tunnel
2. **Check hyper/HTTP serving** - The cell spawns a hyper server on the tunnel stream; maybe it errors
3. **Compare cell state** - Why does the cell behave differently right after a build?
4. **Test on Linux** - Determine if this is macOS-specific
