use datatest_stable::Utf8Path;
use facet_svg::{
    Svg,
    facet_assert::{SameOptions, assert_same_with},
    facet_xml,
};
use std::process::Command;

/// Path to the C pikchr binary (built from vendor/pikchr-c)
const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/pikchr");

/// Tolerance for floating-point comparisons (pikchr uses single precision)
/// Use 0.05 to allow for rounding differences between C and Rust implementations
const FLOAT_TOLERANCE: f64 = 0.05;

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

    let output = child
        .wait_with_output()
        .expect("failed to wait on C pikchr");
    String::from_utf8(output.stdout).expect("C pikchr output not UTF-8")
}

/// Extract SVG portion from output (skipping any print statements before it)
fn extract_svg(output: &str) -> Option<&str> {
    if let Some(start) = output.find("<svg") {
        if let Some(end) = output.rfind("</svg>") {
            return Some(&output[start..end + 6]);
        }
    }
    None
}

/// Parse SVG string into typed Svg struct
fn parse_svg(svg: &str) -> Result<Svg, String> {
    // Extract just the SVG portion in case there's print output before it
    let svg_only = extract_svg(svg).unwrap_or(svg);
    facet_xml::from_str(svg_only).map_err(|e| format!("XML parse error: {}", e))
}

/// Options for SVG comparison with float tolerance
fn svg_compare_options() -> SameOptions {
    SameOptions::new().float_tolerance(FLOAT_TOLERANCE)
}

fn test_pikchr_file(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = std::fs::read_to_string(path)?;

    // Get expected output from C implementation
    let c_output = run_c_pikchr(&source);

    // Check if C output is an error (contains ERROR:)
    let c_is_error = c_output.contains("ERROR:");

    // Check if C output contains SVG
    let c_has_svg = c_output.contains("<svg");

    // Get output from our Rust implementation
    let rust_result = pikru::pikchr(&source);

    match rust_result {
        Ok(rust_output) => {
            if c_is_error {
                panic!(
                    "C pikchr produced error but Rust succeeded for {}\nC output: {}",
                    path, c_output
                );
            }

            // If neither has SVG, compare raw output (e.g., empty diagram comments)
            if !c_has_svg && !rust_output.contains("<svg") {
                // Both produced non-SVG output - compare as strings (trimmed)
                let c_trimmed = c_output.trim();
                let rust_trimmed = rust_output.trim();
                assert_eq!(
                    c_trimmed, rust_trimmed,
                    "Non-SVG output mismatch for {}\nC: {}\nRust: {}",
                    path, c_trimmed, rust_trimmed
                );
                return Ok(());
            }

            // Parse both SVGs
            let c_svg =
                parse_svg(&c_output).map_err(|e| format!("Failed to parse C SVG: {}", e))?;
            let rust_svg =
                parse_svg(&rust_output).map_err(|e| format!("Failed to parse Rust SVG: {}", e))?;

            // Compare using facet-assert with float tolerance
            assert_same_with!(
                c_svg,
                rust_svg,
                svg_compare_options(),
                "SVG mismatch for {}",
                path
            );
        }
        Err(e) => {
            if c_is_error {
                // Both implementations produced errors - this is expected for tests with `error` statements
                return Ok(());
            }
            panic!("Rust implementation failed for {}: {}", path, e);
        }
    }

    Ok(())
}

/// Compare two pikchr sources semantically
#[allow(dead_code)]
fn assert_svg_matches(source: &str, context: &str) {
    let c_out = run_c_pikchr(source);
    let rust_out = pikru::pikchr(source).expect("rust pikchr failed");

    let c_svg = parse_svg(&c_out).expect("Failed to parse C SVG");
    let rust_svg = parse_svg(&rust_out).expect("Failed to parse Rust SVG");

    assert_same_with!(c_svg, rust_svg, svg_compare_options(), "{}", context);
}

#[test]
fn even_with_horizontal_matches_c() {
    let source = r#"
        box "A"
        B: box at (2,1)
        arrow right even with B
    "#;
    assert_svg_matches(source, "right even with should align x to target");
}

#[test]
fn until_even_vertical_matches_c() {
    let source = r#"
        box "A"
        B: box at (1,2)
        arrow down until even with B
    "#;
    assert_svg_matches(source, "until even with should align y to target");
}

datatest_stable::harness! {
    { test = test_pikchr_file, root = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/tests"), pattern = r"\.pikchr$" },
}
