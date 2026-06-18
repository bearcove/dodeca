fn main() {
    // Test hex colors
    let input = r#"
box "Coral" fill #ff7f50
arrow
box "Deep Sky" fill #00bfff
arrow
box "Gold" fill #ffd700
arrow
box "Custom" fill #8a2be2 color #ffffff
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
