//! SVG comparison utilities for testing pikchr output.
//!
//! This crate provides shared comparison logic used by both the test harness
//! and the xtask visual comparison tool.

use camino::Utf8Path;
use facet_assert::{SameOptions, SameReport, check_same_with_report};
use facet_svg::{Svg, facet_xml};
use std::fs;
use std::process::Command;

/// Tolerance for floating-point comparisons (pikchr uses single precision)
/// Keep this tight so genuine geometry differences don't get masked.
/// A value of 0.002 px covers formatting/rounding noise while
/// catching mis-chopped endpoints like autochop02.
pub const FLOAT_TOLERANCE: f64 = 0.002;

/// Similarity threshold for tree-based element matching.
/// Elements with structural similarity >= this threshold are paired for
/// inline field-level diffing rather than shown as remove+add.
/// A value of 0.5 means elements sharing 50%+ of their structure are paired.
pub const SIMILARITY_THRESHOLD: f64 = 0.5;

/// Result of comparing two pikchr outputs
#[derive(Debug, Clone)]
pub enum CompareResult {
    /// Both outputs match (within tolerance)
    Match,
    /// Both produced errors, and they match
    BothErrorMatch,
    /// Both produced errors, but they differ
    BothErrorMismatch {
        c_output: String,
        rust_output: String,
    },
    /// C errored but Rust succeeded
    CErrorRustSuccess { c_output: String },
    /// Rust errored but C succeeded
    RustErrorCSuccess { rust_error: String },
    /// Both produced non-SVG output that matches
    NonSvgMatch,
    /// Both produced non-SVG output that differs
    NonSvgMismatch {
        c_output: String,
        rust_output: String,
    },
    /// SVG outputs differ
    SvgMismatch { details: String },
    /// Failed to parse one of the SVGs
    ParseError { details: String },
}

impl CompareResult {
    pub fn is_match(&self) -> bool {
        matches!(
            self,
            CompareResult::Match
                | CompareResult::BothErrorMatch
                | CompareResult::BothErrorMismatch { .. }
                | CompareResult::NonSvgMatch
        )
    }
}

/// Extract SVG portion from output (skipping any print statements before it)
pub fn extract_svg(output: &str) -> Option<&str> {
    // Try lowercase <svg> first (standard/C implementation)
    if let Some(start) = output.find("<svg") {
        if let Some(end) = output.rfind("</svg>") {
            return Some(&output[start..end + 6]);
        }
    }
    // Try capitalized <Svg> (facet-xml output before namespace fix)
    if let Some(start) = output.find("<Svg") {
        if let Some(end) = output.rfind("</Svg>") {
            return Some(&output[start..end + 6]);
        }
    }
    None
}

/// Extract text before SVG (print output, comments, etc.)
pub fn extract_pre_svg_text(output: &str) -> Option<&str> {
    if let Some(start) = output.find("<svg") {
        let text = output[..start].trim();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// Parse SVG string into typed Svg struct
pub fn parse_svg(svg: &str) -> Result<Svg, String> {
    // Extract just the SVG portion in case there's print output before it
    let svg_only = extract_svg(svg).unwrap_or(svg);
    facet_xml::from_str(svg_only)
        .map_err(|e| format!("XML parse error: {:?}", miette::Report::new(e)))
}

/// Options for SVG comparison with float tolerance and tree-based similarity
pub fn svg_compare_options() -> SameOptions {
    SameOptions::new()
        .float_tolerance(FLOAT_TOLERANCE)
        .similarity_threshold(SIMILARITY_THRESHOLD)
}

/// Check if output represents an error
pub fn is_error_output(output: &str) -> bool {
    output.contains("ERROR:") || (!output.contains("<svg") && !output.contains("<!--"))
}

/// Compare two pikchr outputs with tolerance
///
/// This is the single source of truth for comparing C and Rust pikchr outputs.
pub fn compare_outputs(c_output: &str, rust_output: &str, rust_is_err: bool) -> CompareResult {
    let c_is_error = c_output.contains("ERROR:");
    let c_has_svg = c_output.contains("<svg");
    let c_has_comment = c_output.contains("<!--");

    let rust_has_svg = rust_output.contains("<svg");
    let rust_has_comment = rust_output.contains("<!--");

    // Handle error cases
    if rust_is_err {
        if c_is_error {
            // Both errored - compare error messages
            if c_output.trim() == rust_output.trim() {
                return CompareResult::BothErrorMatch;
            } else {
                return CompareResult::BothErrorMismatch {
                    c_output: c_output.to_string(),
                    rust_output: rust_output.to_string(),
                };
            }
        } else {
            return CompareResult::RustErrorCSuccess {
                rust_error: rust_output.to_string(),
            };
        }
    }

    if c_is_error {
        return CompareResult::CErrorRustSuccess {
            c_output: c_output.to_string(),
        };
    }

    // Neither errored - compare outputs
    // If neither has SVG, compare raw output (e.g., empty diagram comments)
    if !c_has_svg && !rust_has_svg {
        if c_has_comment || rust_has_comment {
            // Both produced non-SVG output - compare as strings (trimmed)
            if c_output.trim() == rust_output.trim() {
                return CompareResult::NonSvgMatch;
            } else {
                return CompareResult::NonSvgMismatch {
                    c_output: c_output.to_string(),
                    rust_output: rust_output.to_string(),
                };
            }
        }
    }

    // Parse both SVGs
    let c_svg = match parse_svg(c_output) {
        Ok(svg) => svg,
        Err(e) => {
            return CompareResult::ParseError {
                details: format!("Failed to parse C SVG: {}", e),
            };
        }
    };

    let rust_svg = match parse_svg(rust_output) {
        Ok(svg) => svg,
        Err(e) => {
            return CompareResult::ParseError {
                details: format!("Failed to parse Rust SVG: {}", e),
            };
        }
    };

    // Compare using facet-assert with float tolerance
    match check_same_with_report(&c_svg, &rust_svg, svg_compare_options()) {
        SameReport::Same => CompareResult::Match,
        SameReport::Different(report) => {
            let xml_diff = report.render_ansi_xml();
            CompareResult::SvgMismatch { details: xml_diff }
        }
        SameReport::Opaque { type_name } => CompareResult::SvgMismatch {
            details: format!("Opaque type comparison not supported: {}", type_name),
        },
    }
}

/// Write debug SVGs for a test so we can inspect C vs Rust output.
///
/// Writes to `{debug_dir}/{test_name}-c.svg` and `{debug_dir}/{test_name}-rust.svg`.
/// Creates the debug directory if it doesn't exist.
pub fn write_debug_svgs(debug_dir: &Utf8Path, test_name: &str, c_output: &str, rust_output: &str) {
    // Create debug directory if it doesn't exist
    fs::create_dir_all(debug_dir).ok();

    let c_svg = extract_svg(c_output).unwrap_or("<!-- No SVG found -->");
    let rust_svg = extract_svg(rust_output).unwrap_or("<!-- No SVG found -->");

    let c_file = debug_dir.join(format!("{}-c.svg", test_name));
    let rust_file = debug_dir.join(format!("{}-rust.svg", test_name));

    if let Err(e) = fs::write(&c_file, c_svg) {
        eprintln!("Warning: Failed to write {}: {}", c_file, e);
    }
    if let Err(e) = fs::write(&rust_file, rust_svg) {
        eprintln!("Warning: Failed to write {}: {}", rust_file, e);
    }
}

/// Run the C pikchr implementation and return its SVG output.
pub fn run_c_pikchr(c_pikchr_path: &Utf8Path, source: &str) -> String {
    use std::io::Write;

    let mut child = Command::new(c_pikchr_path.as_str())
        .arg("--svg-only")
        .arg("/dev/stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run C pikchr");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();

    let output = child
        .wait_with_output()
        .expect("failed to wait on C pikchr");
    String::from_utf8_lossy(&output.stdout).to_string()
}
