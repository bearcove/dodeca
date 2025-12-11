use datatest_stable::Utf8Path;
use pikru::compare::{CompareResult, compare_outputs};
use std::process::Command;
use std::sync::Once;

static INIT_TRACING: Once = Once::new();

fn init_tracing() {
    INIT_TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .init();
    });
}

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
    init_tracing();

    // Install miette fancy GraphicalReporter for better error display
    miette::set_hook(Box::new(
        |_| Box::new(miette::GraphicalReportHandler::new()),
    ))
    .ok();

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

    if result.is_match() {
        return Ok(());
    }

    match result {
        CompareResult::SvgMismatch { details } => {
            panic!("SVG mismatch for {}\n{}", path, details);
        }
        CompareResult::ParseError { details } => {
            panic!("Parse error for {}\n{}", path, details);
        }
        CompareResult::BothErrorMismatch {
            c_output,
            rust_output,
        } => {
            panic!(
                "Both implementations errored differently for {}\nC: {}\nRust: {}",
                path, c_output, rust_output
            );
        }
        CompareResult::CErrorRustSuccess { c_output } => {
            panic!(
                "C pikchr errored but Rust succeeded for {}\nC output: {}",
                path, c_output
            );
        }
        CompareResult::RustErrorCSuccess { rust_error } => {
            panic!(
                "Rust errored but C succeeded for {}\nRust error: {}",
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
        CompareResult::Match
        | CompareResult::BothErrorMatch
        | CompareResult::NonSvgMatch => unreachable!(),
    }
}

datatest_stable::harness! {
    { test = test_pikchr_file, root = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/tests"), pattern = r"\.pikchr$" },
}

#[test]
fn z_generate_comparison_html() {
    // Named with 'z_' prefix to run after other tests
    // Generate comparison HTML after running all pikchr tests
    match pikru::generate_comparison_html() {
        Ok(_) => println!("✅ Comparison HTML generated successfully"),
        Err(e) => {
            // Don't fail tests if comparison generation fails, just warn
            eprintln!("⚠️  Warning: Failed to generate comparison HTML: {}", e);
        }
    }
}
