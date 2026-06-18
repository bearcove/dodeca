fn main() {
    // Test multi-segment line with then clauses
    let input = r#"
box "Start"
line right 50 then down 50 then right 50 ->
box "End"
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
