fn main() {
    // Test chop attribute (shortens lines at shape boundaries)
    let input = r#"
A: circle "A"
B: circle "B" at A + (150, 0)
arrow from A to B "no chop"
C: circle "C" at A + (0, 100)
D: circle "D" at C + (150, 0)
arrow from C to D chop "chop"
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
