fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(std::io::stderr)
        .init();

    let input = std::env::args()
        .nth(1)
        .map(|path| std::fs::read_to_string(&path).expect("Failed to read file"))
        .unwrap_or_else(|| {
            r#"box "A"
box "B" at 1in ne of A"#
                .to_string()
        });

    match pikru::pikchr(&input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
