#![no_main]

//! Apply parity fuzzer.
//!
//! Compares native hotmeal patch application against browser hotmeal-wasm.
//! Both apply the same patches, and we compare DOM trees at each step.

use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| target(data));

fn target(data: &[u8]) {
    INIT.call_once(|| {
        facet_testhelpers::setup();
        common::init_thrall_quiet();
    });

    let Some((full_a, full_b)) = common::prepare_html_inputs(data) else {
        return;
    };

    // Compute patches using hotmeal
    let full_a = StrTendril::from(full_a.clone());
    let full_b = StrTendril::from(full_b.clone());

    let patches = match hotmeal::diff_html(&full_a, &full_b) {
        Ok(p) => p,
        Err(err) => panic!("hotmeal failed to diff {err}"),
    };

    // Send to browser worker for roundtrip
    let Some(browser_result) = common::test_roundtrip(full_a.to_string(), full_b.to_string())
    else {
        return;
    };

    // Skip cases where browser parsed to empty
    if browser_result.normalized_old.trim().is_empty() {
        return;
    }

    // Apply patches natively, capturing the full trace
    let normalized_old_tendril = StrTendril::from(browser_result.normalized_old.clone());
    let mut native_doc = hotmeal::parse(&normalized_old_tendril);
    let Some(native_trace) = common::PatchTrace::capture(&mut native_doc, &patches) else {
        return; // Skip documents without a body
    };

    // Convert browser result to our trace format
    let browser_trace = common::PatchTrace::from(&browser_result);

    // Compare traces
    if let Some(mismatch) = common::compare_traces(&native_trace, &browser_trace) {
        eprintln!("\n========== APPLY PARITY MISMATCH ==========");
        eprintln!("Input A: {:?}", full_a);
        eprintln!("Input B: {:?}", full_b);
        eprintln!("\n{}", mismatch);
        eprintln!("\n--- Full Native Trace ---");
        eprintln!("{}", native_trace);
        eprintln!("\n--- Full Browser Trace ---");
        eprintln!("{}", browser_trace);
        eprintln!("============================================\n");
        panic!("Apply parity mismatch!");
    }
}
