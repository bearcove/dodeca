fn main() {
    // Test text styling: bold, italic, mono, big, small
    let input = r#"
box "Normal text"
arrow
box "Bold text" bold
arrow
box "Italic text" italic
arrow
box "Bold Italic" bold italic
arrow
box "Monospace" mono
arrow
box "BIG TEXT" big
arrow
box "small text" small
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
