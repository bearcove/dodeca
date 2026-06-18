fn main() {
    // Test dot objects (small filled circle markers)
    let input = r#"
box "Start"
dot
arrow right 50
dot
arrow right 50
dot color Red
arrow right 50
dot fill Blue
arrow right 50
box "End"
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
