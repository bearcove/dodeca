use datatest_stable::Utf8Path;
use pikru::compare::{CompareResult, compare_outputs};
use std::process::Command;

/// Path to the C pikchr binary (built from vendor/pikchr-c)
const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/pikchr");

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

fn test_pikchr_file(path: &Utf8Path) -> datatest_stable::Result<()> {
    let source = std::fs::read_to_string(path)?;

    // Get expected output from C implementation
    let c_output = run_c_pikchr(&source);

    // Get output from our Rust implementation
    let rust_result = pikru::pikchr(&source);

    let (rust_output, rust_is_err) = match rust_result {
        Ok(s) => (s, false),
        Err(e) => (format!("Error: {}", e), true),
    };

    // Use shared comparison logic
    let result = compare_outputs(&c_output, &rust_output, rust_is_err);

    match result {
        CompareResult::Match | CompareResult::BothErrorMatch | CompareResult::NonSvgMatch => Ok(()),
        CompareResult::BothErrorMismatch {
            c_output,
            rust_output,
        } => {
            panic!(
                "Both implementations errored but with different messages for {}\nC: {}\nRust: {}",
                path, c_output, rust_output
            );
        }
        CompareResult::CErrorRustSuccess { c_output } => {
            panic!(
                "C pikchr produced error but Rust succeeded for {}\nC output: {}",
                path, c_output
            );
        }
        CompareResult::RustErrorCSuccess { rust_error } => {
            panic!(
                "Rust implementation failed but C succeeded for {}\nRust error: {}",
                path, rust_error
            );
        }
        CompareResult::NonSvgMismatch {
            c_output,
            rust_output,
        } => {
            panic!(
                "Non-SVG output mismatch for {}\nC: {}\nRust: {}",
                path, c_output, rust_output
            );
        }
        CompareResult::SvgMismatch { details } => {
            panic!("SVG mismatch for {}\n{}", path, details);
        }
        CompareResult::ParseError { details } => {
            panic!("Parse error for {}\n{}", path, details);
        }
    }
}

datatest_stable::harness! {
    { test = test_pikchr_file, root = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/tests"), pattern = r"\.pikchr$", exclude = r"expr\.pikchr" },
}
