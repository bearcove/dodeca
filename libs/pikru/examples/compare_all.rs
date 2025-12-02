fn main() {
    let test_dir = std::path::Path::new("../pikchr/tests");
    let mut success = 0;
    let mut empty = 0;
    let mut error = 0;
    
    let mut results: Vec<(String, usize)> = Vec::new();
    
    for entry in std::fs::read_dir(test_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().map(|e| e == "pikchr").unwrap_or(false) {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let input = std::fs::read_to_string(&path).unwrap();
            
            match pikru::pikchr(&input) {
                Ok(svg) => {
                    let elements = svg.matches("<circle").count()
                        + svg.matches("<rect").count()
                        + svg.matches("<line").count()
                        + svg.matches("<path").count()
                        + svg.matches("<text").count()
                        + svg.matches("<polygon").count()
                        + svg.matches("<ellipse").count()
                        + svg.matches("<polyline").count();
                    
                    if elements > 0 {
                        success += 1;
                        results.push((name, elements));
                    } else {
                        empty += 1;
                    }
                }
                Err(_) => {
                    error += 1;
                }
            }
        }
    }
    
    println!("=== PIKRU RENDERING STATUS ===");
    println!("Total test files: {}", success + empty + error);
    println!("  Renders with content: {} ({:.0}%)", success, 100.0 * success as f64 / (success + empty + error) as f64);
    println!("  Renders empty: {}", empty);
    println!("  Errors: {}", error);
    println!();
    
    // Show some examples
    results.sort_by(|a, b| b.1.cmp(&a.1));
    println!("Top 10 by element count:");
    for (name, count) in results.iter().take(10) {
        println!("  {}: {} elements", name, count);
    }
}
