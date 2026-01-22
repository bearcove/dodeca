use divan::{Bencher, black_box};

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
    html.replacen("<div", "<div class=\"modified\"", 1)
}

// Full hot-reload cycle: parse old, parse new, diff, apply patches
#[divan::bench]
fn hot_reload_small(bencher: Bencher) {
    let modified = modify_html(SMALL_HTML);
    bencher.bench_local(|| {
        let mut old = hotmeal::parse_untyped(black_box(SMALL_HTML));
        let new = hotmeal::parse_untyped(black_box(&modified));
        let patches = hotmeal::diff::diff_elements(&old, &new).unwrap();
        hotmeal::diff::apply_patches(&mut old, &patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_medium(bencher: Bencher) {
    let modified = modify_html(MEDIUM_HTML);
    bencher.bench_local(|| {
        let mut old = hotmeal::parse_untyped(black_box(MEDIUM_HTML));
        let new = hotmeal::parse_untyped(black_box(&modified));
        let patches = hotmeal::diff::diff_elements(&old, &new).unwrap();
        hotmeal::diff::apply_patches(&mut old, &patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_large(bencher: Bencher) {
    let modified = modify_html(LARGE_HTML);
    bencher.bench_local(|| {
        let mut old = hotmeal::parse_untyped(black_box(LARGE_HTML));
        let new = hotmeal::parse_untyped(black_box(&modified));
        let patches = hotmeal::diff::diff_elements(&old, &new).unwrap();
        hotmeal::diff::apply_patches(&mut old, &patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_xlarge(bencher: Bencher) {
    let modified = modify_html(XLARGE_HTML);
    bencher.bench_local(|| {
        let mut old = hotmeal::parse_untyped(black_box(XLARGE_HTML));
        let new = hotmeal::parse_untyped(black_box(&modified));
        let patches = hotmeal::diff::diff_elements(&old, &new).unwrap();
        hotmeal::diff::apply_patches(&mut old, &patches).unwrap();
        black_box(old);
    });
}
