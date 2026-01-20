//! Simple benchmark binary for profiling HTML diff with samply/perf.
//!
//! Usage:
//!   cargo run --release -p html-diff-tests --bin bench-diff --features matching-stats

use std::time::Instant;

fn main() {
    let old_html = include_str!("../../fixtures/primer-old.html");
    let new_html = include_str!("../../fixtures/primer-new.html");

    println!("Old HTML: {} bytes", old_html.len());
    println!("New HTML: {} bytes", new_html.len());

    let start = Instant::now();
    let patches = html_diff_tests::diff_html_debug(old_html, new_html, true).unwrap();
    let elapsed = start.elapsed();

    println!("\n=== Diff completed in {:?} ===", elapsed);
    println!("DOM Patches: {}", patches.len());

    println!("\n=== DOM Patches ===");
    for patch in &patches {
        println!("{:?}", patch);
    }
}
