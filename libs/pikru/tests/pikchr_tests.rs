use camino::Utf8Path;
use datatest_stable;
use pikru_compare::{CompareResult, compare_outputs, run_c_pikchr, write_debug_svgs};
use std::sync::Once;

static INIT_TRACING: Once = Once::new();

fn init_tracing() {
    INIT_TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .without_time()
            .with_level(false)
            .with_target(false)
            .init();
    });
}

/// Path to the C pikchr binary (built from vendor/pikchr-c)
const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/pikchr");

/// Debug SVG output directory
const DEBUG_SVG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/debug-svg");

fn test_pikchr_file(path: &Utf8Path) -> datatest_stable::Result<()> {
    init_tracing();

    // Install miette fancy GraphicalReporter for better error display
    miette::set_hook(Box::new(
        |_| Box::new(miette::GraphicalReportHandler::new()),
    ))
    .ok();

    let source = std::fs::read_to_string(path)?;

    // Get expected output from C implementation
    let c_pikchr_path = Utf8Path::new(C_PIKCHR);
    let c_output = run_c_pikchr(c_pikchr_path, &source);

    // Get output from our Rust implementation
    let rust_result = pikru::pikchr(&source);

    let (rust_output, rust_is_err) = match rust_result {
        Ok(s) => (s, false),
        Err(e) => (format!("Error: {}", e), true),
    };

    // Extract test name from path (e.g., "test23" from "test23.pikchr")
    let test_name = path.file_stem().unwrap_or("unknown");

    // Always write debug SVGs so we can inspect them
    let debug_dir = Utf8Path::new(DEBUG_SVG_DIR);
    write_debug_svgs(debug_dir, test_name, &c_output, &rust_output);

    // Use shared comparison logic (visual comparison with SSIM)
    let result = compare_outputs(&c_output, &rust_output, rust_is_err);

    if result.is_match() {
        return Ok(());
    }

    match result {
        CompareResult::SvgMismatch { ssim, details } => {
            panic!(
                "SVG mismatch for {} (SSIM: {:.6})\n{}",
                path, ssim, details
            );
        }
        CompareResult::ParseError { details } => {
            panic!("Parse error for {}\n{}", path, details);
        }
        CompareResult::RenderError { details } => {
            panic!("Render error for {}\n{}", path, details);
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
