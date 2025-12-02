fn main() {
    let input = std::fs::read_to_string("../pikchr/tests/test03.pikchr").unwrap();
    let rust_svg = pikru::pikchr(&input).unwrap();
    
    println!("SVG length: {}", rust_svg.len());
    
    // Try parsing
    match roxmltree::Document::parse(&rust_svg) {
        Ok(doc) => {
            let mut count = 0;
            for node in doc.descendants() {
                if node.is_element() {
                    let tag = node.tag_name().name();
                    if matches!(tag, "rect" | "circle" | "line" | "text" | "polygon" | "path") {
                        count += 1;
                    }
                }
            }
            println!("Elements found: {}", count);
        }
        Err(e) => println!("Parse error: {}", e),
    }
}
