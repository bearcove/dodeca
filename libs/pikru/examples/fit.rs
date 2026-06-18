fn main() {
    // Test fit attribute (auto-size boxes to text)
    let input = r#"
box "Short"
arrow
box "This is a longer text" fit
arrow
box "Two" "Lines" fit
arrow
box "Multi-line" "longer text here" "third line" fit
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
