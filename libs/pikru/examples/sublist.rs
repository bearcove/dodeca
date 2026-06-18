fn main() {
    // Test sublist rendering (nested diagrams)
    let input = r#"
box "Main"
arrow
[
    box "Nested 1"
    arrow
    box "Nested 2"
]
arrow
box "After"
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
