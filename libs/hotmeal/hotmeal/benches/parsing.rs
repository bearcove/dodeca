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

#[divan::bench]
fn parse_small(bencher: Bencher) {
    bencher.bench_local(|| {
        let doc = hotmeal::arena_dom::parse(black_box(SMALL_HTML));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_medium(bencher: Bencher) {
    bencher.bench_local(|| {
        let doc = hotmeal::arena_dom::parse(black_box(MEDIUM_HTML));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_large(bencher: Bencher) {
    bencher.bench_local(|| {
        let doc = hotmeal::arena_dom::parse(black_box(LARGE_HTML));
        black_box(doc);
    });
}

#[divan::bench]
fn parse_xlarge(bencher: Bencher) {
    bencher.bench_local(|| {
        let doc = hotmeal::arena_dom::parse(black_box(XLARGE_HTML));
        black_box(doc);
    });
}
