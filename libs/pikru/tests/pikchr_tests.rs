use datatest_stable::Utf8Path;
use std::process::Command;

/// Path to the C pikchr binary (built from ../pikchr)
const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../pikchr/pikchr");

/// Run the C pikchr implementation and return its SVG output
fn run_c_pikchr(source: &str) -> String {
    let mut child = Command::new(C_PIKCHR)
        .arg("--svg-only")
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run C pikchr");

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("failed to wait on C pikchr");
    String::from_utf8(output.stdout).expect("C pikchr output not UTF-8")
}

fn test_pikchr_file(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = std::fs::read_to_string(path)?;

    // Get expected output from C implementation
    let c_output = run_c_pikchr(&source);

    // Get output from our Rust implementation
    let rust_result = pikru::pikchr(&source);

    match rust_result {
        Ok(rust_output) => {
            // Compare outputs (ignoring the date attribute which changes)
            let c_normalized = normalize_svg(&c_output);
            let rust_normalized = normalize_svg(&rust_output);

            if c_normalized != rust_normalized {
                panic!(
                    "Output mismatch for {}:\n\n--- C output ---\n{}\n\n--- Rust output ---\n{}",
                    path, c_output, rust_output
                );
            }
        }
        Err(e) => {
            // For now, just note that Rust implementation isn't done yet
            panic!("Rust implementation failed for {}: {}", path, e);
        }
    }

    Ok(())
}

/// Normalize SVG for comparison by removing volatile attributes like dates
fn normalize_svg(svg: &str) -> String {
    // Remove data-pikchr-date attribute which changes on every run
    let re = regex_lite::Regex::new(r#" data-pikchr-date="[^"]*""#).unwrap();
    re.replace_all(svg, "").to_string()
}

datatest_stable::harness! {
    { test = test_pikchr_file, root = concat!(env!("CARGO_MANIFEST_DIR"), "/../pikchr/tests"), pattern = r"\.pikchr$" },
}
