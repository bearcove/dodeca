fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "../pikchr/tests/test01.pikchr".to_string());
    let input = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to read file: {}", path));
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
            println!("{}", svg);
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}
