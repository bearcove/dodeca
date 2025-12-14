# Strace SHM transport syscalls

Trace doorbell and futex syscalls to debug transport hangs.

## Attach to running process

```bash
strace -ff -p <pid> -e trace=sendto,recvfrom,epoll_wait,epoll_ctl,futex -s 0
```

## Start process with strace

```bash
cd /home/amos/bearcove/picante && strace -ff -o /tmp/ddc-trace -e trace=sendto,recvfrom,epoll_wait,epoll_ctl,futex -s 0 -- systemd-run --user --scope -p MemoryMax=4G /home/amos/bearcove/dodeca/target/debug/ddc build
```

## What to look for

### Host side (when sending request to plugin):
- `sendto(fd, ...)` on the doorbell fd after enqueue
- Should see 1 byte sent

### Plugin side (when receiving):
- `epoll_wait(...)` returning with doorbell fd readable
- `recvfrom(fd, ...)` draining the doorbell
- If stuck in `epoll_wait` but host sent → missed wakeup bug

### Futex operations:
- `futex(addr, FUTEX_WAKE, ...)` for signaling
- `futex(addr, FUTEX_WAIT, ...)` for waiting on ring full/empty

## Minimal trace (just doorbells)

```bash
strace -ff -p <pid> -e trace=sendto,recvfrom -s 0
```
