#![no_main]

//! Apply parity fuzzer.
//!
//! Compares native hotmeal patch application against browser hotmeal-wasm.
//! Both apply the SAME patches to the SAME initial tree, and we compare DOM trees at each step.
//!
//! If html5ever and browser parse the input differently, we skip (that's a parse parity issue).

use browser_proto::OwnedPatches;
use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| target(data));

fn target(data: &[u8]) {
    INIT.call_once(|| {
        unsafe {
            std::env::set_var("FACET_LOG", "warn");
        }
        facet_testhelpers::setup();
        common::init_thrall_quiet();
    });

    let Some((full_a, full_b)) = common::prepare_html_inputs(data) else {
        return;
    };

    // Parse A with html5ever
    let full_a = StrTendril::from(full_a.clone());
    let full_b = StrTendril::from(full_b.clone());
    let mut native_doc = hotmeal::parse(&full_a);

    // Get html5ever's initial tree
    let Some(native_initial_tree) = common::document_body_to_dom_node(&native_doc) else {
        return; // Skip documents without a body
    };

    // Compute patches using hotmeal
    let patches = match hotmeal::diff_html(&full_a, &full_b) {
        Ok(p) => p,
        Err(err) => panic!("hotmeal failed to diff {err}"),
    };

    // Skip if patches contain invalid attr/tag names for DOM APIs
    // (html5ever error recovery can create these, but they can't be set via setAttribute)
    if !common::patches_are_valid_for_dom(&patches) {
        return;
    }

    // Convert patches to owned (static lifetime) for sending to browser
    let owned_patches: Vec<hotmeal::Patch<'static>> =
        patches.iter().map(|p| p.clone().into_owned()).collect();

    // Send the SAME patches to browser for application
    let Some(browser_result) =
        common::apply_patches(full_a.to_string(), OwnedPatches(owned_patches))
    else {
        return;
    };

    // Skip cases where browser parsed to empty
    if browser_result.normalized_old_html.trim().is_empty() {
        return;
    }

    // Compare initial trees - if they differ, html5ever and browser parsed differently
    // That's a parse parity issue, not an apply parity issue - skip it
    if native_initial_tree != browser_result.initial_dom_tree {
        return;
    }

    // Apply patches natively to html5ever's parse (same tree we computed patches from)
    let Some(native_trace) = common::PatchTrace::capture(&mut native_doc, &patches) else {
        return;
    };

    // Convert browser result to our trace format
    let browser_trace = common::PatchTrace::from(&browser_result);

    // Compare traces
    if let Some(mismatch) = common::compare_traces(&native_trace, &browser_trace) {
        eprintln!("\n========== APPLY PARITY MISMATCH ==========");
        eprintln!("Input A: {:?}", full_a);
        eprintln!("Input B: {:?}", full_b);
        eprintln!("\n{}", mismatch);
        eprintln!("\n--- Interleaved Trace ---");
        common::print_interleaved_traces(&native_trace, &browser_trace);
        eprintln!("============================================\n");
        panic!("Apply parity mismatch!");
    }
}
