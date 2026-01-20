//! Debug text change diffing

fn main() {
    // Test case: roundtrip failure case
    println!("=== Case: complex roundtrip ===");
    let old = "<html><body><div><p>A</p></div><span>A</span></body></html>";
    let new = r#"<html><body><div> <span class="a">0</span></div></body></html>"#;
    let patches = html_diff_tests::diff_html_debug(old, new, true).unwrap();
    println!("Patches: {:?}\n", patches);
}
