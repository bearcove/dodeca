# Inspect SHM hub file

Examine the shared memory hub file for debugging.

## Find the hub file

```bash
ls -la /tmp/dodeca-hub-*.shm
```

## Check file size and mapping

```bash
stat /tmp/dodeca-hub-*.shm
```

## Hexdump header (first 256 bytes)

```bash
xxd -l 256 /tmp/dodeca-hub-*.shm
```

## Check which processes have it mapped

```bash
fuser /tmp/dodeca-hub-*.shm 2>/dev/null
```

Or more detailed:
```bash
lsof /tmp/dodeca-hub-*.shm
```

## Notes

- The hub file contains:
  - HubHeader with magic, version, peer count
  - Per-peer entries with ring offsets and status
  - Size class headers for the allocator
  - Extent data with slot metadata and payload data

- Ring header layout (192 bytes each):
  - visible_head (u64) + padding
  - tail (u64) + padding
  - capacity (u32) + padding

- Key offsets can be found in rapace-transport-shm/src/hub_layout.rs
