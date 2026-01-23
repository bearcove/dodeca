//! Roundtrip tests for HTML diff using datatest-stable.
//!
//! Each test case is a file in `tests/roundtrip-cases/` with format:
//! ```
//! <old HTML>
//! ===
//! <new HTML>
//! ```
//!
//! The test verifies: apply(old, diff(old, new)) == new

use hotmeal::diff;
use hotmeal::parse;
use std::path::Path;

fn run_roundtrip_test(path: &Path) -> datatest_stable::Result<()> {
    facet_testhelpers::setup();

    let content = std::fs::read_to_string(path)?;
    let parts: Vec<&str> = content.split("\n===\n").collect();

    if parts.len() != 2 {
        return Err(format!(
            "Test file must have exactly one '===' separator, found {} parts",
            parts.len()
        )
        .into());
    }

    let old = parts[0].trim();
    let new = parts[1].trim();

    let old_doc = parse(old);
    let new_doc = parse(new);

    let patches = diff(&old_doc, &new_doc).map_err(|e| format!("diff failed: {e:?}"))?;

    let mut tree = parse(old);
    tree.apply_patches(patches)
        .map_err(|e| format!("apply failed: {e:?}"))?;
    let result = tree.to_html();

    let expected = new_doc.to_html();

    if result != expected {
        return Err(format!(
            "Roundtrip failed!\nOld: {old}\nNew: {new}\nResult: {result}\nExpected: {expected}"
        )
        .into());
    }

    Ok(())
}

datatest_stable::harness! {
    { test = run_roundtrip_test, root = "tests/roundtrip-cases", pattern = r".*\.html$" },
}
