fn main() {
    // Test from/to positioning with connections
    let input = r#"
A: box "A"
arrow
B: box "B"
arrow
C: box "C"

# Arrow from A's east to C's west (skipping B)
arrow from A.e to C.w color Red dashed

# Arrow from B's south going down
arrow from B.s down 50 color Blue
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
