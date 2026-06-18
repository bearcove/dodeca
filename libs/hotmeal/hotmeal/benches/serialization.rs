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

#[divan::bench]
fn serialize_small(bencher: Bencher) {
    let tendril = StrTendril::from(SMALL_HTML);
    let doc = hotmeal::parse(&tendril);
    bencher.bench_local(|| {
        let html = black_box(&doc).to_html();
        black_box(html);
    });
}

#[divan::bench]
fn serialize_medium(bencher: Bencher) {
    let tendril = StrTendril::from(MEDIUM_HTML);
    let doc = hotmeal::parse(&tendril);
    bencher.bench_local(|| {
        let html = black_box(&doc).to_html();
        black_box(html);
    });
}

#[divan::bench]
fn serialize_large(bencher: Bencher) {
    let tendril = StrTendril::from(LARGE_HTML);
    let doc = hotmeal::parse(&tendril);
    bencher.bench_local(|| {
        let html = black_box(&doc).to_html();
        black_box(html);
    });
}

#[divan::bench]
fn serialize_xlarge(bencher: Bencher) {
    let tendril = StrTendril::from(XLARGE_HTML);
    let doc = hotmeal::parse(&tendril);
    bencher.bench_local(|| {
        let html = black_box(&doc).to_html();
        black_box(html);
    });
}
