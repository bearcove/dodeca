# SHM Redesign Handoff

## Current State

We've been debugging SHM deadlocks and memory issues in dodeca's plugin system.

### What We Fixed
1. **Config in header**: SHM config (ring_capacity, slot_size, slot_count) is now stored in the file header. Plugins use `open_file_auto()` to discover config automatically.
2. **In-process Notify**: Added `tokio::sync::Notify` alongside futex for faster in-process wakeups when waiting for slots.

### The Problem We Hit
Font decompression hangs because:
- Compressed fonts: ~1.5MB (Iosevka)
- **Decompressed fonts: ~10MB**
- Current slot size: 2MB (was 512KB before)

We can't just keep bumping slot size because:
- 17 plugins × 64MB = 1GB+ just for SHM mappings
- Most messages are tiny (< 1KB), only fonts need huge slots
- Fixed slots waste massive amounts of memory

## Plan B: Redesign

### 1. Shared SHM Between Host and ALL Plugins

Current architecture:
```
Host ←→ [SHM file] ←→ Plugin A
Host ←→ [SHM file] ←→ Plugin B
Host ←→ [SHM file] ←→ Plugin C
... (17 separate SHM files, 17 separate slot pools)
```

New architecture:
```
Host ←→ [Single large SHM file] ←→ All Plugins
```

Benefits:
- One slot pool shared across all plugins
- Much better memory utilization
- Slots can be larger without multiplying by 17

### 2. Dynamic Allocation for Responses

Current: Fixed-size slots (all same size)
```
[16MB slot][16MB slot][16MB slot][16MB slot]
```

New: Variable-size allocation from a shared pool
```
[  1KB  ][  10MB  ][  512B  ][  2MB  ]...
```

Could use:
- Bump allocator with periodic compaction
- Or linked list of variable-size chunks
- Or arena with size classes (1KB, 16KB, 256KB, 4MB, 16MB)

### Key Files

**Rapace (SHM transport)**:
- `/home/amos/bearcove/rapace/crates/rapace-transport-shm/src/session.rs` - SHM session creation
- `/home/amos/bearcove/rapace/crates/rapace-transport-shm/src/layout.rs` - Memory layout, slot allocation
- `/home/amos/bearcove/rapace/crates/rapace-transport-shm/src/transport.rs` - Frame send/recv

**Dodeca (plugin host)**:
- `/home/amos/bearcove/dodeca/crates/dodeca/src/plugins.rs` - Plugin launching, SHM config
- `/home/amos/bearcove/dodeca/crates/dodeca-plugin-runtime/src/lib.rs` - Plugin side

### Test Command

```bash
# Clean SHM files first
rm -f /tmp/dodeca-*.shm

# Build with 4GB memory limit
systemd-run --user --scope -p MemoryMax=4G ./target/debug/ddc build docs
```

The font decompression test (in `mods/mod-fonts/src/main.rs`) shows fontcull works fine:
```
Font size: 1506136 bytes
Decompress took: 56.005474ms
Decompressed to 9814676 bytes
```

### Commits Made This Session

**rapace** (main branch):
- `b09231d` - feat(transport-shm): store config in header, add open_file_auto

**dodeca** (memory-mystery branch):
- `f476c9e` - refactor: use open_file_auto for SHM config discovery
- (uncommitted) - slot size bump to 16MB (revert this, it's not the right fix)

### Next Steps

1. Revert the 16MB slot size change in dodeca
2. Design shared SHM architecture in rapace
3. Implement dynamic/variable allocation in rapace
4. Update dodeca to use single shared SHM for all plugins
