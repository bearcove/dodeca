use std::hint::black_box;

const XXLARGE_HTML: &str = include_str!("../tests/fixtures/xxl.html");

fn modify_html(html: &str) -> String {
    html.replacen("<div", "<div class=\"modified\"", 1)
}

fn main() {
    let modified = modify_html(XXLARGE_HTML);

    // Run 100 iterations for profiling
    for _ in 0..100 {
        let mut old = hotmeal::parse(black_box(XXLARGE_HTML));
        let new = hotmeal::parse(black_box(&modified));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(&old);
    }
}
