# Claude Code Notes

## Git commands

Always use `--no-pager` BEFORE the git command to avoid blocking on interactive pager:

```bash
git --no-pager log -10
git --no-pager diff
git --no-pager show
```

## Testing

**DO NOT** run the full test suite with `cargo test` - it times out and hangs.

Run specific tests only:
```bash
cargo test test01 -- --nocapture
cargo test test12 -- --nocapture
```

Never try to count/grep test results from the full suite - it will hang.
