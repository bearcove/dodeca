//! SVG comparison utilities for testing pikchr output.
//!
//! This crate provides shared comparison logic used by both the test harness
//! and the xtask visual comparison tool.
//!
//! Primary comparison is visual (render with resvg, compare with SSIM).
//! Falls back to structural comparison (facet-assert) for detailed diff output
//! when visual comparison fails.

use camino::Utf8Path;
use facet_assert::{SameOptions, SameReport, check_same_with_report};
use facet_format_svg::Svg;
use std::fs;
use std::process::Command;

/// SSIM threshold for visual comparison.
/// 1.0 = identical, 0.0 = completely different.
/// 0.999 allows for minor anti-aliasing differences from arc rendering, etc.
pub const SSIM_THRESHOLD: f64 = 0.999;

/// Render size for visual comparison (pixels).
/// Larger = more accurate but slower.
pub const RENDER_SIZE: u32 = 800;

/// Tolerance for floating-point comparisons in structural diff.
/// Used when SSIM fails and we need detailed diff output.
pub const FLOAT_TOLERANCE: f64 = 0.002;

/// Similarity threshold for tree-based element matching in structural diff.
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
    /// SVG outputs differ (includes SSIM score and structural diff)
    SvgMismatch { ssim: f64, details: String },
    /// Failed to parse one of the SVGs
    ParseError { details: String },
    /// Failed to render one of the SVGs
    RenderError { details: String },
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
    if let Some(start) = output.find("<svg")
        && let Some(end) = output.rfind("</svg>")
    {
        return Some(&output[start..end + 6]);
    }
    // Try capitalized <Svg> (facet-xml output before namespace fix)
    if let Some(start) = output.find("<Svg")
        && let Some(end) = output.rfind("</Svg>")
    {
        return Some(&output[start..end + 6]);
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

/// Parse SVG string into typed Svg struct (for structural comparison)
pub fn parse_svg(svg: &str) -> Result<Svg, String> {
    let svg_only = extract_svg(svg).unwrap_or(svg);
    facet_format_svg::from_str(svg_only).map_err(|e| format!("XML parse error: {:?}", e))
}

/// Options for SVG structural comparison with float tolerance
pub fn svg_compare_options() -> SameOptions {
    SameOptions::new()
        .float_tolerance(FLOAT_TOLERANCE)
        .similarity_threshold(SIMILARITY_THRESHOLD)
}

/// Check if output represents an error
pub fn is_error_output(output: &str) -> bool {
    output.contains("ERROR:") || (!output.contains("<svg") && !output.contains("<!--"))
}

/// Convert HTML named entities to Unicode characters.
/// SVG only defines &lt; &gt; &amp; &quot; &apos; - everything else needs conversion.
fn normalize_html_entities(svg: &str) -> String {
    let mut result = svg.to_string();

    // Common HTML entities that might appear in pikchr text
    let entities = [
        ("&sup1;", "¹"),   // superscript 1
        ("&sup2;", "²"),   // superscript 2
        ("&sup3;", "³"),   // superscript 3
        ("&lambda;", "λ"), // Greek lambda
        ("&alpha;", "α"),
        ("&beta;", "β"),
        ("&gamma;", "γ"),
        ("&delta;", "δ"),
        ("&epsilon;", "ε"),
        ("&pi;", "π"),
        ("&sigma;", "σ"),
        ("&omega;", "ω"),
        ("&deg;", "°"),
        ("&plusmn;", "±"),
        ("&times;", "×"),
        ("&divide;", "÷"),
        ("&ne;", "≠"),
        ("&le;", "≤"),
        ("&ge;", "≥"),
        ("&infin;", "∞"),
        ("&rarr;", "→"),
        ("&larr;", "←"),
        ("&uarr;", "↑"),
        ("&darr;", "↓"),
        ("&harr;", "↔"),
        ("&nbsp;", " "),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&trade;", "™"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&hellip;", "…"),
        ("&bull;", "•"),
    ];

    for (entity, unicode) in entities {
        result = result.replace(entity, unicode);
    }

    result
}

/// Render SVG to a pixel buffer using resvg
fn render_svg_to_pixels(svg_content: &str) -> Result<image::RgbaImage, String> {
    // Normalize HTML entities to Unicode
    let normalized = normalize_html_entities(svg_content);

    // Parse SVG with usvg
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(&normalized, &options)
        .map_err(|e| format!("Failed to parse SVG: {}", e))?;

    // Get the SVG size and calculate scale to fit RENDER_SIZE
    let svg_size = tree.size();
    let scale = (RENDER_SIZE as f32 / svg_size.width().max(svg_size.height())).min(2.0);

    let width = (svg_size.width() * scale).ceil() as u32;
    let height = (svg_size.height() * scale).ceil() as u32;

    if width == 0 || height == 0 {
        return Err("SVG has zero size".to_string());
    }

    // Create pixmap and render
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "Failed to create pixmap".to_string())?;

    // Fill with white background (like a browser would)
    pixmap.fill(tiny_skia::Color::WHITE);

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Convert to image::RgbaImage
    let img = image::RgbaImage::from_raw(width, height, pixmap.take())
        .ok_or_else(|| "Failed to convert pixmap to image".to_string())?;

    Ok(img)
}

/// Compare two images using SSIM and return the score (1.0 = identical, 0.0 = different)
fn compare_images_ssim(img1: &image::RgbaImage, img2: &image::RgbaImage) -> Result<f64, String> {
    let (w1, h1) = img1.dimensions();
    let (w2, h2) = img2.dimensions();

    // Images need to be the same size for comparison
    let (img1_final, img2_final) = if w1 == w2 && h1 == h2 {
        (img1.clone(), img2.clone())
    } else {
        // Resize both to the larger dimensions to avoid losing detail
        let w = w1.max(w2);
        let h = h1.max(h2);

        let img1_resized =
            image::imageops::resize(img1, w, h, image::imageops::FilterType::Lanczos3);
        let img2_resized =
            image::imageops::resize(img2, w, h, image::imageops::FilterType::Lanczos3);

        (img1_resized, img2_resized)
    };

    // Use rgba_hybrid_compare which handles RGBA properly
    // It does MSSIM on luma, then RMS on U, V, and alpha channels
    let result = image_compare::rgba_hybrid_compare(&img1_final, &img2_final)
        .map_err(|e| format!("Image comparison failed: {:?}", e))?;

    // Score is 1.0 for identical, 0.0 for completely different
    Ok(result.score)
}

/// Compare two pikchr outputs with visual comparison (SSIM) first,
/// falling back to structural diff for details if visual fails.
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
    if !c_has_svg && !rust_has_svg && (c_has_comment || rust_has_comment) {
        if c_output.trim() == rust_output.trim() {
            return CompareResult::NonSvgMatch;
        } else {
            return CompareResult::NonSvgMismatch {
                c_output: c_output.to_string(),
                rust_output: rust_output.to_string(),
            };
        }
    }

    // Extract SVG content
    let c_svg = match extract_svg(c_output) {
        Some(svg) => svg,
        None => {
            return CompareResult::ParseError {
                details: "No SVG found in C output".to_string(),
            };
        }
    };

    let rust_svg = match extract_svg(rust_output) {
        Some(svg) => svg,
        None => {
            return CompareResult::ParseError {
                details: "No SVG found in Rust output".to_string(),
            };
        }
    };

    // Try visual comparison first
    let c_img = match render_svg_to_pixels(c_svg) {
        Ok(img) => img,
        Err(e) => {
            return CompareResult::RenderError {
                details: format!("Failed to render C SVG: {}", e),
            };
        }
    };

    let rust_img = match render_svg_to_pixels(rust_svg) {
        Ok(img) => img,
        Err(e) => {
            return CompareResult::RenderError {
                details: format!("Failed to render Rust SVG: {}", e),
            };
        }
    };

    let ssim = match compare_images_ssim(&c_img, &rust_img) {
        Ok(score) => score,
        Err(e) => {
            return CompareResult::RenderError {
                details: format!("SSIM comparison failed: {}", e),
            };
        }
    };

    // If SSIM is above threshold, it's a match
    if ssim >= SSIM_THRESHOLD {
        return CompareResult::Match;
    }

    // Visual comparison failed - get structural diff for details
    let details = match (parse_svg(c_output), parse_svg(rust_output)) {
        (Ok(c_parsed), Ok(rust_parsed)) => {
            match check_same_with_report(&c_parsed, &rust_parsed, svg_compare_options()) {
                SameReport::Same => {
                    "Structural comparison shows match (but SSIM failed)".to_string()
                }
                SameReport::Different(report) => report.render_ansi_xml(),
                SameReport::Opaque { type_name } => {
                    format!("Opaque type comparison not supported: {}", type_name)
                }
            }
        }
        (Err(e), _) => format!("Failed to parse C SVG for diff: {}", e),
        (_, Err(e)) => format!("Failed to parse Rust SVG for diff: {}", e),
    };

    CompareResult::SvgMismatch { ssim, details }
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
