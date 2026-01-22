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
fn serialize_small(bencher: Bencher) {
    let doc = hotmeal::parse_document(SMALL_HTML);
    let opts = hotmeal::SerializeOptions::default();
    bencher.bench_local(|| {
        let html = hotmeal::serialize_document(black_box(&doc), &opts);
        black_box(html);
    });
}

#[divan::bench]
fn serialize_medium(bencher: Bencher) {
    let doc = hotmeal::parse_document(MEDIUM_HTML);
    let opts = hotmeal::SerializeOptions::default();
    bencher.bench_local(|| {
        let html = hotmeal::serialize_document(black_box(&doc), &opts);
        black_box(html);
    });
}

#[divan::bench]
fn serialize_large(bencher: Bencher) {
    let doc = hotmeal::parse_document(LARGE_HTML);
    let opts = hotmeal::SerializeOptions::default();
    bencher.bench_local(|| {
        let html = hotmeal::serialize_document(black_box(&doc), &opts);
        black_box(html);
    });
}

#[divan::bench]
fn serialize_xlarge(bencher: Bencher) {
    let doc = hotmeal::parse_document(XLARGE_HTML);
    let opts = hotmeal::SerializeOptions::default();
    bencher.bench_local(|| {
        let html = hotmeal::serialize_document(black_box(&doc), &opts);
        black_box(html);
    });
}
