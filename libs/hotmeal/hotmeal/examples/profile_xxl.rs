use hotmeal::StrTendril;
use std::hint::black_box;

const XXLARGE_HTML: &str = include_str!("../tests/fixtures/xxl.html");

fn modify_html(html: &str) -> String {
    html.replacen("<div", "<div class=\"modified\"", 1)
}

fn main() {
    let modified = modify_html(XXLARGE_HTML);

    // Create tendrils once outside the loop for fair profiling
    let old_tendril = StrTendril::from(XXLARGE_HTML);
    let new_tendril = StrTendril::from(modified.as_str());

    // Run 100 iterations for profiling
    for _ in 0..100 {
        let mut old = hotmeal::parse(black_box(&old_tendril));
        let new = hotmeal::parse(black_box(&new_tendril));
        let patches = hotmeal::diff(&old, &new).unwrap();
        old.apply_patches(patches).unwrap();
        black_box(&old);
    }
}
