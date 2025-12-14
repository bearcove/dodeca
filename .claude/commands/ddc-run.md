# Run ddc with memory limits

Run ddc through systemd-run with a 4GB memory limit to prevent OOM killing the system.

## Usage

```bash
cd /home/amos/bearcove/picante && systemd-run --user --scope -p MemoryMax=4G /home/amos/bearcove/dodeca/target/debug/ddc $ARGUMENTS
```

Replace `$ARGUMENTS` with the ddc subcommand (e.g., `build`, `serve`).

## Notes

- ALWAYS use systemd-run with MemoryMax=4G when running ddc
- This prevents runaway memory usage from killing the machine
- The process runs in a systemd scope that enforces the limit
