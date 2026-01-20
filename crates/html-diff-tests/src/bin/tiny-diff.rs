//! Tiny HTML diff test to see algorithm behavior on simple cases.

fn main() {
    // Test 1: Simple text change
    println!("=== TEST 1: Simple text change ===");
    let old1 = r#"<html><body><p>Hello</p></body></html>"#;
    let new1 = r#"<html><body><p>Goodbye</p></body></html>"#;
    let patches1 = html_diff_tests::diff_html_debug(old1, new1, false).unwrap();
    println!("Old: {}", old1);
    println!("New: {}", new1);
    println!("Patches ({}):", patches1.len());
    for p in &patches1 {
        println!("  {:?}", p);
    }

    // Test 2: Insert element
    println!("\n=== TEST 2: Insert element ===");
    let old2 = r#"<html><body><p>First</p></body></html>"#;
    let new2 = r#"<html><body><p>First</p><p>Second</p></body></html>"#;
    let patches2 = html_diff_tests::diff_html_debug(old2, new2, false).unwrap();
    println!("Old: {}", old2);
    println!("New: {}", new2);
    println!("Patches ({}):", patches2.len());
    for p in &patches2 {
        println!("  {:?}", p);
    }

    // Test 3: Remove element
    println!("\n=== TEST 3: Remove element ===");
    let old3 = r#"<html><body><p>First</p><p>Second</p></body></html>"#;
    let new3 = r#"<html><body><p>First</p></body></html>"#;
    let patches3 = html_diff_tests::diff_html_debug(old3, new3, false).unwrap();
    println!("Old: {}", old3);
    println!("New: {}", new3);
    println!("Patches ({}):", patches3.len());
    for p in &patches3 {
        println!("  {:?}", p);
    }

    // Test 4: Attribute change
    println!("\n=== TEST 4: Attribute change ===");
    let old4 = r#"<html><body><div class="old">Content</div></body></html>"#;
    let new4 = r#"<html><body><div class="new">Content</div></body></html>"#;
    let patches4 = html_diff_tests::diff_html_debug(old4, new4, false).unwrap();
    println!("Old: {}", old4);
    println!("New: {}", new4);
    println!("Patches ({}):", patches4.len());
    for p in &patches4 {
        println!("  {:?}", p);
    }

    // Test 5: Mixed changes
    println!("\n=== TEST 5: Mixed changes ===");
    let old5 = r#"<html><body><div class="box"><p>One</p><p>Two</p></div></body></html>"#;
    let new5 = r#"<html><body><div class="container"><p>One</p><p>Modified</p><p>Three</p></div></body></html>"#;
    let patches5 = html_diff_tests::diff_html_debug(old5, new5, false).unwrap();
    println!("Old: {}", old5);
    println!("New: {}", new5);
    println!("Patches ({}):", patches5.len());
    for p in &patches5 {
        println!("  {:?}", p);
    }
}
