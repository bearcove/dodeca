fn main() {
    // Test all new features: same, thickness, invisible, close, with

    // 1. Same as - copy properties
    let input1 = r#"
A: box "Original" fill Red width 100
B: box "Same" same as A
"#;
    println!("=== Same As ===");
    match pikru::pikchr(input1) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }

    // 2. Thickness
    let input2 = r#"
box "Thin" thickness 1
arrow
box "Normal" thickness 2
arrow
box "Thick" thickness 4
"#;
    println!("\n=== Thickness ===");
    match pikru::pikchr(input2) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }

    // 3. Invisible
    let input3 = r#"
A: box "Visible"
B: box "Invisible" invisible at A + (100, 0)
C: box "Also Visible" at B + (100, 0)
arrow from A.e to C.w
"#;
    println!("\n=== Invisible ===");
    match pikru::pikchr(input3) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }

    // 4. Close path (filled polygon)
    let input4 = r#"
line right 50 then down 50 then left 50 close fill Yellow
"#;
    println!("\n=== Close Path ===");
    match pikru::pikchr(input4) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }

    // 5. With clause
    let input5 = r#"
A: box "A"
B: box "B" with .n at A.s
"#;
    println!("\n=== With Clause ===");
    match pikru::pikchr(input5) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
