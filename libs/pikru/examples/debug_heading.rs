fn main() {
    // Simple test without chop
    let source1 = r#"
C: box "box"
line from C to 3cm heading 0 from C
"#;

    println!("=== Without chop ===");
    match pikru::pikchr(source1) {
        Ok(svg) => {
            for line in svg.lines() {
                if line.contains("<path") || line.contains("<text") || line.contains("viewBox") {
                    println!("{}", line.trim());
                }
            }
        }
        Err(e) => eprintln!("Render error: {}", e),
    }

    // Now compare with C output
    println!("\n=== C output (without chop) ===");
    let c_out = std::process::Command::new("./vendor/pikchr-c/pikchr")
        .arg("--svg-only")
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    use std::io::Write;
    c_out
        .stdin
        .as_ref()
        .unwrap()
        .write_all(source1.trim().as_bytes())
        .ok();
    let output = c_out.wait_with_output().unwrap();
    let c_svg = String::from_utf8_lossy(&output.stdout);
    for line in c_svg.lines() {
        if line.contains("<path") || line.contains("<text") || line.contains("viewBox") {
            println!("{}", line.trim());
        }
    }
}
