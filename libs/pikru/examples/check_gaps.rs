fn main() {
    let test_dir = std::path::Path::new("../pikchr/tests");

    println!("=== Files that render EMPTY ===");
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
                        + svg.matches("<polygon").count();

                    if elements == 0 {
                        // Show first non-comment line
                        let first_line = input
                            .lines()
                            .filter(|l| {
                                !l.trim().is_empty()
                                    && !l.trim().starts_with('#')
                                    && !l.trim().starts_with("//")
                            })
                            .next()
                            .unwrap_or("");
                        println!(
                            "  {}: \"{}\"",
                            name,
                            first_line.chars().take(50).collect::<String>()
                        );
                    }
                }
                Err(e) => {
                    println!(
                        "ERROR {}: {}",
                        name,
                        e.to_string().lines().next().unwrap_or("")
                    );
                }
            }
        }
    }
}
