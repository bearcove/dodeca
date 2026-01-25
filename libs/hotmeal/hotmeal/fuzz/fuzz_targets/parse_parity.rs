#![no_main]

//! Parser parity fuzzer.
//!
//! Compares html5ever parsing (via hotmeal) against browser DOMParser.
//! Both should produce identical DOM trees.

use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use similar::{ChangeTag, TextDiff};
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    INIT.call_once(|| {
        facet_testhelpers::setup();
        common::init_thrall_quiet();
    });

    let Ok(html) = std::str::from_utf8(data) else {
        return;
    };

    // Skip empty or null-containing inputs
    if html.is_empty() || html.contains('\0') {
        return;
    }

    // Skip inputs with LF-CR (\n\r) sequence - Chrome has a bug where it normalizes
    // LF-CR to a single newline, but per INFRA spec it should become LF-LF.
    // Firefox and Safari are spec-compliant, html5ever is spec-compliant.
    // See: https://infra.spec.whatwg.org/#normalize-newlines
    if html.contains("\n\r") {
        return;
    }

    // Wrap in full HTML document with DOCTYPE for no-quirks mode
    let full_html = format!("<!DOCTYPE html><html><body>{}</body></html>", html);
    let tendril = StrTendril::from(full_html.as_str());

    // Parse with html5ever
    let doc = hotmeal::parse(&tendril);
    let Some(html5ever_tree) = common::document_body_to_dom_node(&doc) else {
        return; // Skip documents without a body
    };

    // Parse with browser
    let Some(browser_tree) = common::parse_to_dom(full_html) else {
        return;
    };

    // Compare
    if html5ever_tree != browser_tree {
        eprintln!("\n========== PARSER MISMATCH ==========");
        eprintln!("Input: {:?}", html);
        eprintln!("\n--- html5ever tree ---");
        eprintln!("{}", html5ever_tree);
        eprintln!("\n--- browser tree ---");
        eprintln!("{}", browser_tree);
        eprintln!("\n--- diff ---");
        print_diff(&html5ever_tree.to_string(), &browser_tree.to_string());
        eprintln!("=====================================\n");
        panic!("Parser mismatch detected!");
    }
});

fn print_diff(expected: &str, actual: &str) {
    let diff = TextDiff::from_lines(expected, actual);
    for change in diff.iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m"),
            ChangeTag::Insert => ("+", "\x1b[32m"),
            ChangeTag::Equal => (" ", ""),
        };
        eprint!("{}{}{}\x1b[0m", color, sign, change.value());
    }
}
