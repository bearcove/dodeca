#![no_main]

//! Native diff+apply fuzzer.
//!
//! Tests that hotmeal can diff two DOMs and apply patches correctly.

use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    INIT.call_once(|| {
        unsafe {
            std::env::set_var("FACET_LOG", "warn");
        }
        facet_testhelpers::setup();
    });

    let Some((full_a, full_b)) = common::prepare_html_inputs(data) else {
        return;
    };

    let a_tendril = StrTendril::from(full_a.clone());
    let b_tendril = StrTendril::from(full_b.clone());

    let patches = match hotmeal::diff_html(&a_tendril, &b_tendril) {
        Ok(p) => p,
        Err(err) => panic!("hotmeal failed to diff: {err}"),
    };

    // Capture the full trace
    let mut patched = hotmeal::parse(&a_tendril);
    let doc_b = hotmeal::parse(&b_tendril);
    let Some(trace) = common::PatchTrace::capture(&mut patched, &patches) else {
        return; // Skip documents without a body
    };

    // Check for failures
    if !trace.all_succeeded() {
        eprintln!("Patch application failed!\n");
        eprintln!("Input A: {:?}", full_a);
        eprintln!("Input B: {:?}", full_b);
        eprintln!("\n{}", trace);
        panic!("apply_patches must always succeed");
    }

    // Compare body contents
    let patched_body = patched.to_body_html();
    let expected_body = doc_b.to_body_html();

    if patched_body != expected_body {
        eprintln!("Roundtrip mismatch!\n");
        eprintln!("Input A: {:?}", full_a);
        eprintln!("Input B: {:?}", full_b);
        eprintln!("\nExpected body: {:?}", expected_body);
        eprintln!("Got body: {:?}", patched_body);
        eprintln!("\nFull trace:\n{}", trace);
        panic!("Patched body content should match target body content");
    }
});
