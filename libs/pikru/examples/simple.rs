fn main() {
    let input = r#"box "A"
box "B" at 1in ne of A"#;

    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
