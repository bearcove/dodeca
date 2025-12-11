//! SVG comparison utilities for testing pikchr output.
//!
//! This module provides shared comparison logic used by both the test harness
//! and the xtask visual comparison tool.

use facet_assert::{SameOptions, SameReport, check_same_with_report};
use facet_svg::{Svg, facet_xml};

/// Tolerance for floating-point comparisons (pikchr uses single precision)
/// Keep this tight so genuine geometry differences don't get masked.
/// A value of 0.002 px covers formatting/rounding noise while
/// catching mis-chopped endpoints like autochop02.
pub const FLOAT_TOLERANCE: f64 = 0.002;

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
            CompareResult::Match | CompareResult::BothErrorMatch | CompareResult::NonSvgMatch
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

/// Options for SVG comparison with float tolerance
pub fn svg_compare_options() -> SameOptions {
    SameOptions::new().float_tolerance(FLOAT_TOLERANCE)
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
