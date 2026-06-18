fn main() {
    // Test rounded corners on boxes
    let input = r#"
box "Normal"
arrow
box "Rounded" radius 10
arrow
box "More Round" radius 20
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
