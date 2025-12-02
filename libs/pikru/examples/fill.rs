fn main() {
    // Test fill colors on shapes
    let input = r#"
box "No fill"
arrow
box "Red fill" fill Red
arrow
box "Blue" fill Blue color White
arrow
circle "Green" fill Green
arrow
box "Yellow" fill Yellow
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
