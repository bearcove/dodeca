fn main() {
    let input = r#"
box "Box"
arrow
cylinder "Cylinder"
arrow
file "File"
arrow
oval "Oval"
arrow
circle "Circle"
arrow
ellipse "Ellipse"
arrow
diamond "Diamond"
"#;
    match pikru::pikchr(input) {
        Ok(svg) => println!("{}", svg),
        Err(e) => eprintln!("Error: {}", e),
    }
}
