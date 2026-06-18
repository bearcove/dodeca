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
fn parse_small(bencher: Bencher) {
    let tendril = StrTendril::from(SMALL_HTML);
    bencher.bench_local(|| {
        let doc = hotmeal::parse(black_box(&tendril));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_medium(bencher: Bencher) {
    let tendril = StrTendril::from(MEDIUM_HTML);
    bencher.bench_local(|| {
        let doc = hotmeal::parse(black_box(&tendril));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_large(bencher: Bencher) {
    let tendril = StrTendril::from(LARGE_HTML);
    bencher.bench_local(|| {
        let doc = hotmeal::parse(black_box(&tendril));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_xlarge(bencher: Bencher) {
    let tendril = StrTendril::from(XLARGE_HTML);
    bencher.bench_local(|| {
        let doc = hotmeal::parse(black_box(&tendril));
        black_box(doc);
    });
}
