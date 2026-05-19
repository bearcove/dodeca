//! Replays saved crash reproducers through the exact fuzz-target code path.
//!
//! Each file in `regressions/<target>/` is a libFuzzer input that once crashed
//! a fuzz target. We feed every one back through the same entry point the
//! fuzzer uses (`common::apply_roundtrip`) so these stay fixed and any
//! regression turns this test red under `cargo nextest`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

#[path = "../fuzz_targets/common/mod.rs"]
mod common;

#[test]
fn apply_regressions() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("regressions/apply");

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();

    assert!(
        !entries.is_empty(),
        "no regression inputs found in {}",
        dir.display()
    );

    let mut failures = Vec::new();
    for path in &entries {
        let data =
            std::fs::read(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        if catch_unwind(AssertUnwindSafe(|| common::apply_roundtrip(&data))).is_err() {
            failures.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} apply regressions still crash: {:?}",
        failures.len(),
        entries.len(),
        failures
    );
}
