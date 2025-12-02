fn main() {
    // Test text positioning
    let input = r#"
box "Center" "Line 2"
arrow
box "Title" above "Content" "Footer" below
arrow
box "Left" ljust
arrow
box "Right" rjust
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
