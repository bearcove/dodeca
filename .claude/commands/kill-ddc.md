# Kill all ddc processes

Kill ddc and all its plugin processes.

## Command

```bash
pkill -9 -f "ddc" 2>/dev/null; pkill -9 -f "dodeca-mod" 2>/dev/null; echo "Killed all ddc processes"
```

## Verify

```bash
pgrep -f "ddc|dodeca-mod" || echo "All ddc processes killed"
```

## Notes

- Use this before starting a fresh debug session
- Kills both the main ddc process and all plugin subprocesses
- The hub SHM file in /tmp/dodeca-hub-*.shm can be left behind (cleaned up on next run)
