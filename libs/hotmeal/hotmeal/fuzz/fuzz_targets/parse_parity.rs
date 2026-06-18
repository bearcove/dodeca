#![no_main]

//! Parser parity fuzzer.
//!
//! Compares html5ever fragment parsing against browser innerHTML.
//! Both should produce identical DOM trees.

use hotmeal::StrTendril;
use libfuzzer_sys::fuzz_target;
use similar::{ChangeTag, TextDiff};
use std::sync::Once;

mod common;

static INIT: Once = Once::new();

fuzz_target!(|data: &[u8]| {
    INIT.call_once(|| {
        common::setup_tracing();
        common::init_thrall_quiet();
    });

    let Some(html) = common::prepare_single_html_input(data) else {
        return;
    };

    let tendril = StrTendril::from(html.as_str());

    // Parse with html5ever as body fragment (matches innerHTML behavior)
    let doc = hotmeal::parse_body_fragment(&tendril);
    let Some(html5ever_tree) = common::document_body_to_dom_node(&doc) else {
        return; // Skip documents without a body
    };

    // Parse with browser innerHTML
    let Some(browser_tree) = common::parse_to_dom(html.clone()) else {
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
