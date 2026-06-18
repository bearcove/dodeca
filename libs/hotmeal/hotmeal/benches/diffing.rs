use divan::{Bencher, black_box};
use hotmeal::StrTendril;

fn main() {
    divan::main();
}

// Test fixtures of different sizes
const SMALL_HTML: &str =
    include_str!("../tests/fixtures/https_quora.com_What-is-markup-in-HTML.html"); // 8KB
const MEDIUM_HTML: &str =
    include_str!("../tests/fixtures/https_markdownguide.org_basic-syntax.html"); // 68KB
const LARGE_HTML: &str =
    include_str!("../tests/fixtures/https_developer.mozilla.org_en-US_docs_Web_HTML.html"); // 172KB
const XLARGE_HTML: &str = include_str!("../tests/fixtures/https_fasterthanli.me.html"); // 340KB

/// Helper to make a small change to HTML
fn modify_html(html: &str) -> String {
    // Simple modification: add a class to the first div
    html.replacen("<div", "<div class=\"modified\"", 1)
}

// Diff benchmarks: parse old + new + compute diff
#[divan::bench]
fn diff_small(bencher: Bencher) {
    let modified = modify_html(SMALL_HTML);
    let old_tendril = StrTendril::from(SMALL_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_medium(bencher: Bencher) {
    let modified = modify_html(MEDIUM_HTML);
    let old_tendril = StrTendril::from(MEDIUM_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_large(bencher: Bencher) {
    let modified = modify_html(LARGE_HTML);
    let old_tendril = StrTendril::from(LARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_xlarge(bencher: Bencher) {
    let modified = modify_html(XLARGE_HTML);
    let old_tendril = StrTendril::from(XLARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        black_box(patches);
    });
}

// Diff only (assume already parsed)
#[divan::bench]
fn diff_only_small(bencher: Bencher) {
    let modified = modify_html(SMALL_HTML);
    let old_tendril = StrTendril::from(SMALL_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    let old = hotmeal::parse(&old_tendril);
    let new = hotmeal::parse(&new_tendril);

    bencher.bench_local(|| {
        let patches = hotmeal::diff(black_box(&old), black_box(&new)).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_only_medium(bencher: Bencher) {
    let modified = modify_html(MEDIUM_HTML);
    let old_tendril = StrTendril::from(MEDIUM_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    let old = hotmeal::parse(&old_tendril);
    let new = hotmeal::parse(&new_tendril);

    bencher.bench_local(|| {
        let patches = hotmeal::diff(black_box(&old), black_box(&new)).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_only_large(bencher: Bencher) {
    let modified = modify_html(LARGE_HTML);
    let old_tendril = StrTendril::from(LARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    let old = hotmeal::parse(&old_tendril);
    let new = hotmeal::parse(&new_tendril);

    bencher.bench_local(|| {
        let patches = hotmeal::diff(black_box(&old), black_box(&new)).unwrap();
        black_box(patches);
    });
}

#[divan::bench]
fn diff_only_xlarge(bencher: Bencher) {
    let modified = modify_html(XLARGE_HTML);
    let old_tendril = StrTendril::from(XLARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    let old = hotmeal::parse(&old_tendril);
    let new = hotmeal::parse(&new_tendril);

    bencher.bench_local(|| {
        let patches = hotmeal::diff(black_box(&old), black_box(&new)).unwrap();
        black_box(patches);
    });
}
