#![no_main]

//! Apply parity fuzzer.
//!
//! Compares native hotmeal patch application against browser hotmeal-wasm.
//! Both apply the SAME patches to the SAME initial tree, and we compare DOM trees at each step.
//!
//! Uses fragment parsing (like innerHTML) for parity with browser behavior.

use browser_proto::OwnedPatches;
use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| target(data));

fn target(data: &[u8]) {
    INIT.call_once(|| {
        common::setup_tracing();
        common::init_thrall_quiet();
    });

    tracing::info!("IT BEGINS");

    let Some((html_a, html_b)) = common::prepare_html_inputs(data) else {
        tracing::warn!("prepare_html_inputs filtered out");
        return;
    };

    // Parse as body fragments (matches browser innerHTML behavior)
    let tendril_a = StrTendril::from(html_a.clone());
    let tendril_b = StrTendril::from(html_b.clone());
    let mut native_doc = hotmeal::parse_body_fragment(&tendril_a);
    let new_doc = hotmeal::parse_body_fragment(&tendril_b);

    // Get html5ever's initial tree
    let Some(native_initial_tree) = common::document_body_to_dom_node(&native_doc) else {
        tracing::warn!("no body in native doc");
        return;
    };

    // Compute patches using hotmeal diff on parsed documents
    let patches = match hotmeal::diff(&native_doc, &new_doc) {
        Ok(p) => p,
        Err(err) => panic!("hotmeal failed to diff {err}"),
    };

    // Skip if patches contain invalid attr/tag names for DOM APIs
    // (html5ever error recovery can create these, but they can't be set via setAttribute)
    if !common::patches_are_valid_for_dom(&patches) {
        tracing::warn!("patches have invalid attrs/tags for DOM");
        return;
    }

    // Convert patches to owned (static lifetime) for sending to browser
    let owned_patches: Vec<hotmeal::Patch<'static>> =
        patches.iter().map(|p| p.clone().into_owned()).collect();

    // Send the SAME patches to browser for application
    let Some(browser_result) = common::apply_patches(html_a.clone(), OwnedPatches(owned_patches))
    else {
        tracing::warn!("apply_patches returned None");
        return;
    };

    // Skip cases where browser parsed to empty
    if browser_result.normalized_old_html.trim().is_empty() {
        tracing::warn!("browser parsed to empty");
        return;
    }

    // Compare initial trees - if they differ, something is wrong with our setup
    if native_initial_tree != browser_result.initial_dom_tree {
        eprintln!("\n========== INITIAL TREE MISMATCH ==========");
        eprintln!("Input A: {:?}", html_a);
        eprintln!("Input B: {:?}", html_b);
        eprintln!("\n--- Native (html5ever) ---");
        eprintln!("{}", native_initial_tree);
        eprintln!("\n--- Browser (live DOM after innerHTML) ---");
        eprintln!("{}", browser_result.initial_dom_tree);
        eprintln!("============================================\n");
        panic!("Initial tree mismatch - innerHTML round-trip issue?");
    }

    // Apply patches natively to html5ever's parse (same tree we computed patches from)
    let Some(native_trace) = common::PatchTrace::capture(&mut native_doc, &patches) else {
        tracing::warn!("PatchTrace::capture returned None");
        return;
    };

    // Convert browser result to our trace format
    let browser_trace = common::PatchTrace::from(&browser_result);

    // Compare traces
    if let Some(mismatch) = common::compare_traces(&native_trace, &browser_trace) {
        eprintln!("\n========== APPLY PARITY MISMATCH ==========");
        eprintln!("Input A: {:?}", html_a);
        eprintln!("Input B: {:?}", html_b);
        eprintln!("\n{}", mismatch);
        eprintln!("\n--- Interleaved Trace ---");
        common::print_interleaved_traces(&native_trace, &browser_trace);
        eprintln!("============================================\n");
        panic!("Apply parity mismatch!");
    }

    tracing::info!("YESSS A REAL SUCCESS");
}
