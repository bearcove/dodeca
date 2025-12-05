use std::fs;
use std::io::Write;

fn main() {
    let input = r#"C: box "box"
line from C to 3cm heading 0 from C chop;
line from C to 3cm heading 90 from C chop;"#;

    match pikru::pikchr(input) {
        Ok(svg) => {
            // Write to file
            fs::write("debug_minimal_rust.svg", &svg).expect("Failed to write SVG file");
            println!("Generated debug_minimal_rust.svg");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
