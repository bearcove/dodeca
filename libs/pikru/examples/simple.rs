fn main() {
    let input = r#"box "Start"
arrow
box "Middle"
arrow
box "End""#;

    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
