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
const XXLARGE_HTML: &str = include_str!("../tests/fixtures/xxl.html"); // 492KB

/// Helper to make a small change to HTML
fn modify_html(html: &str) -> String {
    html.replacen("<div", "<div class=\"modified\"", 1)
}

// Full hot-reload cycle with arena_dom: parse old, parse new, diff, apply patches
#[divan::bench]
fn hot_reload_small(bencher: Bencher) {
    let modified = modify_html(SMALL_HTML);
    let old_tendril = StrTendril::from(SMALL_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_medium(bencher: Bencher) {
    let modified = modify_html(MEDIUM_HTML);
    let old_tendril = StrTendril::from(MEDIUM_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_large(bencher: Bencher) {
    let modified = modify_html(LARGE_HTML);
    let old_tendril = StrTendril::from(LARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_xlarge(bencher: Bencher) {
    let modified = modify_html(XLARGE_HTML);
    let old_tendril = StrTendril::from(XLARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(old);
    });
}

#[divan::bench]
fn hot_reload_xxlarge(bencher: Bencher) {
    let modified = modify_html(XXLARGE_HTML);
    let old_tendril = StrTendril::from(XXLARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());
    bencher.bench_local(|| {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(old);
    });
}
