fn main() {
    let input = std::fs::read_to_string("../pikchr/tests/test01.pikchr")
        .expect("Failed to read file");
    match pikru::pikchr(&input) {
        Ok(svg) => {
            println!("SVG length: {} bytes", svg.len());
            println!("Elements: circle={} rect={} line={} path={} text={} polygon={}",
                svg.matches("<circle").count(),
                svg.matches("<rect").count(),
                svg.matches("<line").count(),
                svg.matches("<path").count(),
                svg.matches("<text").count(),
                svg.matches("<polygon").count());
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
