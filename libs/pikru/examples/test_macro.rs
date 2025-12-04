fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("vendor/pikchr-c/tests/test63.pikchr");

    let input = std::fs::read_to_string(file).expect("Failed to read file");
    match pikru::pikchr(&input) {
        Ok(svg) => {
            let elements = svg.matches("<rect").count()
                + svg.matches("<circle").count()
                + svg.matches("<text").count()
                + svg.matches("<line").count();
            println!("Success! {} shape elements", elements);
            if elements > 0 && elements < 20 {
                println!("{}", svg);
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
