# Hub Debugging Handoff

## Status: Hub Works, But Hangs on Font Plugin Call

The SHM hub architecture is **working** - plugins connect, register, and communicate. However, there's a hang when calling the font plugin.

## What Works

- Hub SHM created at `/tmp/dodeca-hub-{pid}.shm`
- All 16 plugins connect with peer_ids 0-15
- RPC calls work for: markdown, arborium (syntax highlighting), sass, svgo, css, js, pagefind, image
- TracingConfig set_filter works for all plugins including fonts

## The Bug

The first actual RPC call to `mod-fonts` (`extract_css_from_html`) hangs:

```
mod_fonts_proto: RPC call start service="FontProcessor" method="extract_css_from_html"
```

### Stack Traces (from GDB)

**Host (ddc, PID 338537):**
```
#0  futex_wait () at src/futex.rs:36
#1  {async_fn#0} () at hub_transport.rs:323  <-- Ring full, waiting for space
#2  send_frame()
#3  session.rs:547  <-- RPC client sending request
```

The host is blocked in `send_frame` because the peer's recv ring is **full** (RingError::Full).

**Plugin (dodeca-mod-fonts, PID 338838):**
```
#0  epoll_wait()
#1  tokio runtime idle
```

The plugin is **idle**, waiting for work in epoll. It's not draining its recv ring.

## Root Cause Analysis

1. Host sends RPC request to font plugin's recv ring
2. Ring is full (or becomes full)
3. Host blocks in futex_wait waiting for ring space
4. Plugin should be woken by doorbell to drain ring
5. **BUG: Plugin is NOT being woken** - it's idle in epoll_wait

The doorbell signal isn't reaching the plugin, OR the plugin isn't responding to it.

## Possible Causes

1. **Doorbell mismatch**: Host might be signaling wrong doorbell
2. **Ring never drained**: Plugin's recv_frame loop might not be running
3. **AsyncFd registration issue**: Plugin's doorbell might not be registered with tokio's epoll
4. **FD inheritance issue**: Doorbell FD might be invalid after fork

## Key Files

| File | Description |
|------|-------------|
| `rapace/crates/rapace-transport-shm/src/hub_transport.rs` | Host and peer transport, send_frame/recv_frame |
| `rapace/crates/rapace-transport-shm/src/doorbell.rs` | Socketpair doorbell for cross-process wakeup |
| `rapace/crates/rapace-transport-shm/src/hub_session.rs` | HubHost::add_peer creates doorbells |
| `dodeca/crates/dodeca-cell-runtime/src/lib.rs` | Plugin side creates HubPeerTransport |

## Debugging Commands

```bash
# Find processes
ps aux | grep ddc
ps aux | grep dodeca-mod-fonts

# Get stack traces
sudo gdb -batch -ex "thread apply all bt" -p <PID>

# Check doorbell FDs
ls -la /proc/<plugin_pid>/fd/

# Run with memory protection (ALWAYS!)
systemd-run --user --scope -p MemoryMax=4G ./target/debug/ddc build docs/
```

## What to Investigate Next

1. **Verify doorbell FD is valid in plugin**:
   - Add logging in `Doorbell::from_raw_fd` to print the FD
   - Check `/proc/<pid>/fd/20` exists and is a socket

2. **Check doorbell registration with tokio**:
   - The `AsyncFd::new()` call in `from_raw_fd` should register with epoll
   - Add tracing to see if `wait()` is ever called

3. **Check plugin's recv loop**:
   - The plugin uses `run_plugin!` which calls `session.run()`
   - This should be polling `recv_frame` continuously
   - Add tracing to see if recv_frame is being called

4. **Test doorbell in isolation**:
   - The unit tests in `doorbell.rs` pass
   - But they don't test cross-process behavior

## Build Notes

- `cargo clean` fixed a wild linker issue with mod-sass (stale artifacts)
- Use `cargo xtask build` to build everything
- cell-http and cell-tui were updated to use `create_hub_transport` instead of old `create_shm_transport`
