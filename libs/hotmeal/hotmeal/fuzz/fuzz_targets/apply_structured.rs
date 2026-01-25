#![no_main]

//! Structured DOM roundtrip fuzzer.
//!
//! Uses a structured DOM generator to create realistic HTML documents,
//! then tests native diff+apply roundtrip.

use libfuzzer_sys::fuzz_target;
use std::sync::Once;

mod common;

use common::{FuzzInput, FuzzMode, chaos_nodes_to_html, extended_nodes_to_html};

static INIT: Once = Once::new();

fuzz_target!(|input: FuzzInput| {
    INIT.call_once(|| {
        unsafe {
            std::env::set_var("FACET_LOG", "warn");
        }
        facet_testhelpers::setup();
    });

    let (old_html, new_html) = match &input.mode {
        FuzzMode::Extended {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            extended_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            extended_nodes_to_html(new, new_doctype, false),
        ),
        FuzzMode::Chaos {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            chaos_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            chaos_nodes_to_html(new, new_doctype, false),
        ),
        FuzzMode::Mixed {
            old_doctype,
            old,
            new_doctype,
            new,
        } => (
            extended_nodes_to_html(old, old_doctype, input.add_invalid_nesting),
            chaos_nodes_to_html(new, new_doctype, false),
        ),
    };

    let old_tendril = hotmeal::StrTendril::from(old_html.clone());
    let new_tendril = hotmeal::StrTendril::from(new_html.clone());

    let patches = hotmeal::diff_html(&old_tendril, &new_tendril).expect("diff failed");
    let mut doc = hotmeal::parse(&old_tendril);
    doc.apply_patches(patches.clone()).expect("apply failed");

    let result = doc.to_html_without_doctype();
    let expected_doc = hotmeal::parse(&new_tendril);
    let expected = expected_doc.to_html_without_doctype();

    assert_eq!(
        result, expected,
        "Roundtrip failed!\nOld: {}\nNew: {}\nPatches: {:?}",
        old_html, new_html, patches
    );
});
