# Send SIGUSR1 for hang diagnostics

Send SIGUSR1 to ddc or a plugin to dump transport diagnostics.

## Usage

Find the ddc process:
```bash
pgrep -f "ddc"
```

Send SIGUSR1:
```bash
kill -USR1 <pid>
```

## What it dumps

For the host (ddc):
- Hub allocator slot status (free/allocated/in_flight counts per size class)
- Per-peer ring status: `recv_ring(head=X tail=Y len=Z/256)` and `send_ring(...)`
- Doorbell pending bytes for each peer

For plugins:
- recv_ring and send_ring status
- doorbell_pending bytes

## Interpreting output

- `recv_ring len > 0` but `doorbell_pending=0` on plugin → missed wakeup
- `recv_ring len=0` on plugin when host is waiting → host never enqueued
- `doorbell_pending > 0` on plugin → plugin isn't draining properly
- Many `in_flight` slots → messages stuck in transit
