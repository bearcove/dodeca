fn main() {
    let input = r#"C: box "box"
line from C to 3cm heading 0 from C chop;
line from C to 3cm heading 90 from C chop;
line from C to 3cm heading 180 from C chop;
line from C to 3cm heading 270 from C chop;"#;

    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
