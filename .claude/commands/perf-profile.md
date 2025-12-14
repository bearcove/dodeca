# Profile with perf (frame pointers)

Profile ddc using perf with frame pointer unwinding (faster than DWARF).

## Prerequisites

Ensure dodeca is built with frame pointers:
```bash
RUSTFLAGS="-C force-frame-pointers=yes" cargo build
```

## Record a profile

### Attach to running process:
```bash
perf record -g --call-graph fp -p <pid> -o /tmp/ddc.perf -- sleep 10
```

### Start with profiling:
```bash
cd /home/amos/bearcove/picante && perf record -g --call-graph fp -o /tmp/ddc.perf -- systemd-run --user --scope -p MemoryMax=4G /home/amos/bearcove/dodeca/target/debug/ddc build
```

## Analyze

### Interactive TUI:
```bash
perf report -i /tmp/ddc.perf
```

### Flamegraph (requires inferno or flamegraph.pl):
```bash
perf script -i /tmp/ddc.perf | inferno-collapse-perf | inferno-flamegraph > /tmp/ddc-flamegraph.svg
```

### Top functions:
```bash
perf report -i /tmp/ddc.perf --stdio --no-children | head -50
```

## Notes

- Use `fp` (frame pointers) not `dwarf` for lower overhead
- Frame pointers must be enabled at compile time
- For async code, look for `poll` functions and tokio runtime frames
