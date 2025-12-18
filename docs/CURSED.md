# Cursed: First Test Fails After Build

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

## Next Steps to Investigate

1. Run with `RUST_BACKTRACE=1` to see where exactly the connection reset happens
2. Add tracing to the FD-passing code path in ddc
3. Check if cargo leaves any file descriptors open after build (`lsof`)
4. Try running the test binary under `dtruss` or similar to trace syscalls
5. Check if this reproduces on Linux (not just macOS)

## Workaround

For now, `cargo xtask integration -- --no-build` works reliably after an initial build.
