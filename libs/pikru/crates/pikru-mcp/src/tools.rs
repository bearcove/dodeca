use facet::Facet;
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::{
    CallToolResult, ContentBlock, ImageContent, TextContent, schema_utils::CallToolError,
};
use rust_mcp_sdk::tool_box;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

use crate::handler::PikruPaths;

// Re-export base64 encoding
use base64::Engine;

/// Helper to create CallToolError from a string message
fn tool_error(msg: impl Into<String>) -> CallToolError {
    CallToolError::new(std::io::Error::other(msg.into()))
}

/// Result of running a pikchr test
#[derive(Debug, Facet)]
pub struct TestResult {
    pub test_name: String,
    pub status: String,
    #[facet(skip_unless_truthy)]
    pub c_error: Option<String>,
    #[facet(skip_unless_truthy)]
    pub rust_error: Option<String>,
    pub comparison: Comparison,
    /// Path to full diff file (if there was a mismatch)
    #[facet(skip_unless_truthy)]
    pub diff_file: Option<String>,
    /// Preview of the diff (first ~100 lines)
    #[facet(skip_unless_truthy)]
    pub diff_preview: Option<String>,
}

#[derive(Debug, Facet)]
pub struct Comparison {
    #[facet(skip_unless_truthy)]
    pub ssim: Option<f64>,
    #[facet(skip_unless_truthy)]
    pub pixel_diff: Option<PixelDiff>,
    pub viewbox: ViewboxComparison,
    pub elements: ElementComparison,
    pub text: TextComparison,
}

#[derive(Debug, Facet)]
pub struct PixelDiff {
    pub c_only: u32,
    pub rust_only: u32,
    pub both: u32,
    pub neither: u32,
    pub overlap_pct: f64,
}

#[derive(Debug, Facet)]
pub struct ViewboxComparison {
    #[facet(skip_unless_truthy)]
    pub c: Option<Viewbox>,
    #[facet(skip_unless_truthy)]
    pub rust: Option<Viewbox>,
    #[facet(rename = "match", skip_unless_truthy)]
    pub matches: Option<bool>,
}

#[derive(Debug, Facet, PartialEq)]
pub struct Viewbox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Facet)]
pub struct ElementComparison {
    pub c: ElementCounts,
    pub rust: ElementCounts,
    #[facet(rename = "match")]
    pub matches: bool,
}

#[derive(Debug, Facet, PartialEq, Default)]
pub struct ElementCounts {
    #[facet(skip_unless_truthy)]
    pub circle: u32,
    #[facet(skip_unless_truthy)]
    pub ellipse: u32,
    #[facet(skip_unless_truthy)]
    pub line: u32,
    #[facet(skip_unless_truthy)]
    pub path: u32,
    #[facet(skip_unless_truthy)]
    pub polygon: u32,
    #[facet(skip_unless_truthy)]
    pub polyline: u32,
    #[facet(skip_unless_truthy)]
    pub rect: u32,
    #[facet(skip_unless_truthy)]
    pub text: u32,
}

#[derive(Debug, Facet)]
pub struct TextComparison {
    pub c: Vec<String>,
    pub rust: Vec<String>,
    #[facet(rename = "match")]
    pub matches: bool,
}

#[derive(Debug, Facet)]
pub struct TestListResult {
    pub total: usize,
    pub numbered_tests: Vec<String>,
    pub autochop_tests: Vec<String>,
    pub other_tests: Vec<String>,
}

//====================//
//  ListPikruTests    //
//====================//
#[mcp_tool(
    name = "list_pikru_tests",
    description = "List all available pikru compliance tests. Returns test names grouped by category.",
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListPikruTestsTool {}

impl ListPikruTestsTool {
    pub fn call_tool(&self, paths: &PikruPaths) -> Result<CallToolResult, CallToolError> {
        let tests = get_available_tests(&paths.tests_dir);

        let numbered: Vec<_> = tests
            .iter()
            .filter(|t| t.starts_with("test"))
            .cloned()
            .collect();
        let autochop: Vec<_> = tests
            .iter()
            .filter(|t| t.starts_with("autochop"))
            .cloned()
            .collect();
        let other: Vec<_> = tests
            .iter()
            .filter(|t| !t.starts_with("test") && !t.starts_with("autochop"))
            .cloned()
            .collect();

        let result = TestListResult {
            total: tests.len(),
            numbered_tests: numbered,
            autochop_tests: autochop,
            other_tests: other,
        };

        let json = facet_json::to_string(&result);

        Ok(CallToolResult::text_content(vec![TextContent::from(json)]))
    }
}

//====================//
//  RunPikruTest      //
//====================//
#[mcp_tool(
    name = "run_pikru_test",
    description = "Run a single pikru compliance test comparing C and Rust implementations. Returns side-by-side comparison images and detailed diff information.",
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RunPikruTestTool {
    /// Name of the test (e.g., 'test01', 'autochop02'). Don't include .pikchr extension.
    pub test_name: String,
}

impl RunPikruTestTool {
    pub fn call_tool(&self, paths: &PikruPaths) -> Result<CallToolResult, CallToolError> {
        let test_file = paths.tests_dir.join(format!("{}.pikchr", self.test_name));

        if !test_file.exists() {
            let available = get_available_tests(&paths.tests_dir);
            let hint = available
                .iter()
                .take(10)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(tool_error(format!(
                "Test '{}' not found. Available: {}...",
                self.test_name, hint
            )));
        }

        let source = std::fs::read_to_string(&test_file).map_err(|e| tool_error(e.to_string()))?;

        // Run C pikchr
        let (c_svg, c_error) = run_c_pikchr(&source, &paths.c_pikchr);
        // source is read but not included in result to save tokens

        // Run Rust pikchr
        let (rust_svg, rust_error) = run_rust_pikchr(&test_file, paths);

        // Run the actual cargo test to get match status
        let (status, svg_diff) = run_cargo_test(&self.test_name, &paths.project_root);

        // Parse SVGs for comparison
        let c_viewbox = c_svg.as_ref().and_then(|s| extract_viewbox(s));
        let rust_viewbox = rust_svg.as_ref().and_then(|s| extract_viewbox(s));

        let c_elements = c_svg
            .as_ref()
            .map(|s| count_svg_elements(s))
            .unwrap_or_default();
        let rust_elements = rust_svg
            .as_ref()
            .map(|s| count_svg_elements(s))
            .unwrap_or_default();

        let c_texts = c_svg
            .as_ref()
            .map(|s| extract_text_content(s))
            .unwrap_or_default();
        let rust_texts = rust_svg
            .as_ref()
            .map(|s| extract_text_content(s))
            .unwrap_or_default();

        // Convert SVGs to PNGs for visual comparison
        let c_png = c_svg.as_ref().and_then(|s| svg_to_png(s, 300));
        let rust_png = rust_svg.as_ref().and_then(|s| svg_to_png(s, 300));

        // Calculate pixel diff
        let pixel_diff = match (&c_png, &rust_png) {
            (Some(c), Some(r)) => calculate_pixel_diff(c, r),
            _ => None,
        };

        // Calculate SSIM
        let ssim = match (&c_png, &rust_png) {
            (Some(c), Some(r)) => calculate_ssim(c, r),
            _ => None,
        };

        // Write diff to temp file if present, extract preview
        let (diff_file, diff_preview) = if let Some(ref diff) = svg_diff {
            let diff_path = format!("/tmp/pikru-diff-{}.txt", self.test_name);
            std::fs::write(&diff_path, diff).ok();

            // Extract first ~100 lines as preview
            let preview: String = diff
                .lines()
                .take(100)
                .collect::<Vec<_>>()
                .join("\n");
            let preview = if diff.lines().count() > 100 {
                format!("{}\n... ({} more lines, see {})", preview, diff.lines().count() - 100, diff_path)
            } else {
                preview
            };

            (Some(diff_path), Some(preview))
        } else {
            (None, None)
        };

        let result = TestResult {
            test_name: self.test_name.clone(),
            status,
            c_error,
            rust_error,
            comparison: Comparison {
                ssim,
                pixel_diff,
                viewbox: ViewboxComparison {
                    matches: match (&c_viewbox, &rust_viewbox) {
                        (Some(c), Some(r)) => Some(c == r),
                        _ => None,
                    },
                    c: c_viewbox,
                    rust: rust_viewbox,
                },
                elements: ElementComparison {
                    matches: c_elements == rust_elements,
                    c: c_elements,
                    rust: rust_elements,
                },
                text: TextComparison {
                    matches: c_texts == rust_texts,
                    c: c_texts,
                    rust: rust_texts,
                },
            },
            diff_file,
            diff_preview,
        };

        let json = facet_json::to_string(&result);

        // Build response with text and images
        let mut content: Vec<ContentBlock> = vec![TextContent::from(json).into()];

        // Create side-by-side image
        if let Some(side_by_side) = create_side_by_side(&c_png, &rust_png) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&side_by_side);
            content.push(ImageContent::new(b64, "image/png".to_string(), None, None).into());
        }

        // Create diff image
        if let (Some(c), Some(r)) = (&c_png, &rust_png) {
            if let Some(diff_img) = create_diff_image(c, r) {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&diff_img);
                content.push(ImageContent::new(b64, "image/png".to_string(), None, None).into());
            }
        }

        Ok(CallToolResult {
            content,
            is_error: None,
            meta: None,
            structured_content: None,
        })
    }
}

// Generate the tool box enum
tool_box!(PikruTools, [ListPikruTestsTool, RunPikruTestTool]);

//====================//
//  Helper functions  //
//====================//

fn get_available_tests(tests_dir: &Path) -> Vec<String> {
    let mut tests = Vec::new();
    if let Ok(entries) = std::fs::read_dir(tests_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "pikchr") {
                if let Some(stem) = path.file_stem() {
                    tests.push(stem.to_string_lossy().to_string());
                }
            }
        }
    }
    tests.sort();
    tests
}

fn run_c_pikchr(source: &str, c_pikchr_path: &Path) -> (Option<String>, Option<String>) {
    let output = Command::new(c_pikchr_path)
        .args(["--svg-only", "/dev/stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(source.as_bytes())?;
            }
            child.wait_with_output()
        });

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            if out.status.success() && stdout.contains("<svg") {
                (extract_svg(&stdout), None)
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                (None, Some(format!("C pikchr error: {stderr}")))
            }
        }
        Err(e) => (None, Some(format!("Failed to run C pikchr: {e}"))),
    }
}

fn run_rust_pikchr(test_file: &Path, paths: &PikruPaths) -> (Option<String>, Option<String>) {
    // Shell out to `cargo run --example simple` so we always test current code
    // without needing to rebuild the MCP
    let output = Command::new("cargo")
        .args(["run", "--example", "simple", "--"])
        .arg(test_file)
        .current_dir(&paths.project_root)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            if out.status.success() && stdout.contains("<svg") {
                (extract_svg(&stdout), None)
            } else {
                (None, Some(format!("Rust pikchr error: {}", stderr)))
            }
        }
        Err(e) => (None, Some(format!("Failed to run Rust pikchr: {e}"))),
    }
}

/// Strip ANSI escape sequences from text
fn strip_ansi(text: &str) -> String {
    let bytes = strip_ansi_escapes::strip(text);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn run_cargo_test(test_name: &str, project_root: &Path) -> (String, Option<String>) {
    let output = Command::new("cargo")
        .args([
            "test",
            &format!("{}.pikchr", test_name),
            "--",
            "--nocapture",
        ])
        .current_dir(project_root)
        .output();

    match output {
        Ok(out) => {
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            );

            let status = if out.status.success() {
                "match".to_string()
            } else if combined.contains("SVG mismatch") {
                "mismatch".to_string()
            } else if combined.contains("Parse error") {
                "parse_error".to_string()
            } else {
                "error".to_string()
            };

            // Extract diff output and strip ANSI escapes
            let svg_diff = if let Some(start) = combined.find("SVG mismatch for") {
                let end = combined[start..]
                    .find("\nnote:")
                    .or_else(|| combined[start..].find("\nfailures:"))
                    .map(|i| start + i)
                    .unwrap_or(combined.len());
                Some(strip_ansi(combined[start..end].trim()))
            } else {
                None
            };

            (status, svg_diff)
        }
        Err(e) => (
            "error".to_string(),
            Some(format!("Failed to run test: {e}")),
        ),
    }
}

fn extract_svg(output: &str) -> Option<String> {
    let start = output.find("<svg")?;
    let end = output.rfind("</svg>")?;
    Some(output[start..end + 6].to_string())
}

fn extract_viewbox(svg: &str) -> Option<Viewbox> {
    // Try viewBox attribute
    let re = regex_lite::Regex::new(r#"viewBox=["']([^"']+)["']"#).ok()?;
    if let Some(caps) = re.captures(svg) {
        let parts: Vec<f64> = caps
            .get(1)?
            .as_str()
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        if parts.len() == 4 {
            return Some(Viewbox {
                x: parts[0],
                y: parts[1],
                width: parts[2],
                height: parts[3],
            });
        }
    }

    // Try width/height attributes
    let width_re = regex_lite::Regex::new(r#"width=["']([0-9.]+)"#).ok()?;
    let height_re = regex_lite::Regex::new(r#"height=["']([0-9.]+)"#).ok()?;

    let width: f64 = width_re.captures(svg)?.get(1)?.as_str().parse().ok()?;
    let height: f64 = height_re.captures(svg)?.get(1)?.as_str().parse().ok()?;

    Some(Viewbox {
        x: 0.0,
        y: 0.0,
        width,
        height,
    })
}

fn count_svg_elements(svg: &str) -> ElementCounts {
    let mut counts = ElementCounts::default();

    // Simple regex-based counting
    counts.circle = regex_lite::Regex::new(r"<circle\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.ellipse = regex_lite::Regex::new(r"<ellipse\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.line = regex_lite::Regex::new(r"<line\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.path = regex_lite::Regex::new(r"<path\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.polygon = regex_lite::Regex::new(r"<polygon\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.polyline = regex_lite::Regex::new(r"<polyline\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.rect = regex_lite::Regex::new(r"<rect\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);
    counts.text = regex_lite::Regex::new(r"<text\b")
        .map(|r| r.find_iter(svg).count() as u32)
        .unwrap_or(0);

    counts
}

fn extract_text_content(svg: &str) -> Vec<String> {
    let mut texts = Vec::new();
    let re = regex_lite::Regex::new(r"<text[^>]*>([^<]*)</text>").ok();
    if let Some(re) = re {
        for caps in re.captures_iter(svg) {
            if let Some(text) = caps.get(1) {
                let t = text.as_str().trim();
                if !t.is_empty() {
                    texts.push(t.to_string());
                }
            }
        }
    }
    texts
}

fn svg_to_png(svg: &str, target_width: u32) -> Option<Vec<u8>> {
    // Parse SVG using usvg
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &options).ok()?;

    // Calculate scale to fit target width
    let svg_size = tree.size();
    let scale = target_width as f32 / svg_size.width();
    let width = (svg_size.width() * scale).ceil() as u32;
    let height = (svg_size.height() * scale).ceil() as u32;

    // Create pixmap
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;

    // Fill with white background
    pixmap.fill(tiny_skia::Color::WHITE);

    // Render SVG
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Encode as PNG
    pixmap.encode_png().ok()
}

fn calculate_pixel_diff(c_png: &[u8], rust_png: &[u8]) -> Option<PixelDiff> {
    let c_img = image::load_from_memory(c_png).ok()?.to_rgba8();
    let rust_img = image::load_from_memory(rust_png).ok()?.to_rgba8();

    let width = c_img.width().max(rust_img.width());
    let height = c_img.height().max(rust_img.height());

    let c_resized = image::imageops::resize(&c_img, width, height, image::imageops::Lanczos3);
    let rust_resized = image::imageops::resize(&rust_img, width, height, image::imageops::Lanczos3);

    let mut c_only = 0u32;
    let mut rust_only = 0u32;
    let mut both = 0u32;
    let mut neither = 0u32;

    for y in 0..height {
        for x in 0..width {
            let c_pixel = c_resized.get_pixel(x, y);
            let rust_pixel = rust_resized.get_pixel(x, y);

            let c_gray = (c_pixel[0] as f32 + c_pixel[1] as f32 + c_pixel[2] as f32) / 3.0;
            let rust_gray =
                (rust_pixel[0] as f32 + rust_pixel[1] as f32 + rust_pixel[2] as f32) / 3.0;

            let c_present = c_gray < 250.0 && c_pixel[3] > 128;
            let rust_present = rust_gray < 250.0 && rust_pixel[3] > 128;

            match (c_present, rust_present) {
                (true, true) => both += 1,
                (true, false) => c_only += 1,
                (false, true) => rust_only += 1,
                (false, false) => neither += 1,
            }
        }
    }

    let total_content = c_only + rust_only + both;
    let overlap_pct = if total_content > 0 {
        (100.0 * both as f64) / total_content as f64
    } else {
        100.0
    };

    Some(PixelDiff {
        c_only,
        rust_only,
        both,
        neither,
        overlap_pct: (overlap_pct * 10.0).round() / 10.0,
    })
}

fn calculate_ssim(c_png: &[u8], rust_png: &[u8]) -> Option<f64> {
    let c_img = image::load_from_memory(c_png).ok()?.to_luma8();
    let rust_img = image::load_from_memory(rust_png).ok()?.to_luma8();

    // Resize to same dimensions if needed
    let width = c_img.width().max(rust_img.width());
    let height = c_img.height().max(rust_img.height());

    let c_resized = if c_img.width() != width || c_img.height() != height {
        image::imageops::resize(&c_img, width, height, image::imageops::Lanczos3)
    } else {
        c_img
    };

    let rust_resized = if rust_img.width() != width || rust_img.height() != height {
        image::imageops::resize(&rust_img, width, height, image::imageops::Lanczos3)
    } else {
        rust_img
    };

    // Use image-compare for proper SSIM calculation
    let result = image_compare::gray_similarity_structure(
        &image_compare::Algorithm::MSSIMSimple,
        &c_resized,
        &rust_resized,
    )
    .ok()?;

    Some((result.score * 10000.0).round() / 10000.0)
}

fn create_side_by_side(c_png: &Option<Vec<u8>>, rust_png: &Option<Vec<u8>>) -> Option<Vec<u8>> {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let placeholder_width = 300u32;
    let placeholder_height = 200u32;

    // Load or create placeholder for C image
    let c_img: RgbaImage = if let Some(data) = c_png {
        image::load_from_memory(data).ok()?.to_rgba8()
    } else {
        ImageBuffer::from_pixel(
            placeholder_width,
            placeholder_height,
            Rgba([255, 200, 200, 255]),
        )
    };

    // Load or create placeholder for Rust image
    let rust_img: RgbaImage = if let Some(data) = rust_png {
        image::load_from_memory(data).ok()?.to_rgba8()
    } else {
        ImageBuffer::from_pixel(
            placeholder_width,
            placeholder_height,
            Rgba([255, 200, 200, 255]),
        )
    };

    // Match heights
    let max_height = c_img.height().max(rust_img.height());
    let c_img = if c_img.height() != max_height {
        image::imageops::resize(
            &c_img,
            (c_img.width() as f32 * max_height as f32 / c_img.height() as f32) as u32,
            max_height,
            image::imageops::Lanczos3,
        )
    } else {
        c_img
    };
    let rust_img = if rust_img.height() != max_height {
        image::imageops::resize(
            &rust_img,
            (rust_img.width() as f32 * max_height as f32 / rust_img.height() as f32) as u32,
            max_height,
            image::imageops::Lanczos3,
        )
    } else {
        rust_img
    };

    let label_height = 25u32;
    let gap = 10u32;
    let total_width = c_img.width() + gap + rust_img.width();
    let total_height = max_height + label_height;

    // Create combined image
    let mut combined: RgbaImage =
        ImageBuffer::from_pixel(total_width, total_height, Rgba([255, 255, 255, 255]));

    // Draw blue header for C
    for x in 0..c_img.width() {
        for y in 0..label_height {
            combined.put_pixel(x, y, Rgba([59, 130, 246, 255])); // Blue
        }
    }

    // Draw orange header for Rust
    for x in (c_img.width() + gap)..total_width {
        for y in 0..label_height {
            combined.put_pixel(x, y, Rgba([249, 115, 22, 255])); // Orange
        }
    }

    // Copy C image
    for (x, y, pixel) in c_img.enumerate_pixels() {
        combined.put_pixel(x, y + label_height, *pixel);
    }

    // Copy Rust image
    for (x, y, pixel) in rust_img.enumerate_pixels() {
        combined.put_pixel(x + c_img.width() + gap, y + label_height, *pixel);
    }

    // Encode as PNG
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    combined.write_with_encoder(encoder).ok()?;
    Some(buf)
}

fn create_diff_image(c_png: &[u8], rust_png: &[u8]) -> Option<Vec<u8>> {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let c_img = image::load_from_memory(c_png).ok()?.to_rgba8();
    let rust_img = image::load_from_memory(rust_png).ok()?.to_rgba8();

    let width = c_img.width().max(rust_img.width());
    let height = c_img.height().max(rust_img.height());

    let c_img = image::imageops::resize(&c_img, width, height, image::imageops::Lanczos3);
    let rust_img = image::imageops::resize(&rust_img, width, height, image::imageops::Lanczos3);

    let legend_height = 30u32;
    let mut diff: RgbaImage =
        ImageBuffer::from_pixel(width, height + legend_height, Rgba([255, 255, 255, 255]));

    // Draw legend boxes
    let box_size = 12u32;
    // Blue box for C only
    for x in 10..(10 + box_size) {
        for y in 8..(8 + box_size) {
            diff.put_pixel(x, y, Rgba([59, 130, 246, 255]));
        }
    }
    // Orange box for Rust only
    for x in 90..(90 + box_size) {
        for y in 8..(8 + box_size) {
            diff.put_pixel(x, y, Rgba([249, 115, 22, 255]));
        }
    }
    // Green box for both
    for x in 180..(180 + box_size) {
        for y in 8..(8 + box_size) {
            diff.put_pixel(x, y, Rgba([100, 150, 100, 255]));
        }
    }

    // Create diff visualization
    for y in 0..height {
        for x in 0..width {
            let c_pixel = c_img.get_pixel(x, y);
            let rust_pixel = rust_img.get_pixel(x, y);

            let c_gray = (c_pixel[0] as f32 + c_pixel[1] as f32 + c_pixel[2] as f32) / 3.0;
            let rust_gray =
                (rust_pixel[0] as f32 + rust_pixel[1] as f32 + rust_pixel[2] as f32) / 3.0;

            let c_present = c_gray < 250.0 && c_pixel[3] > 128;
            let rust_present = rust_gray < 250.0 && rust_pixel[3] > 128;

            let color = match (c_present, rust_present) {
                (true, true) => Rgba([100, 150, 100, 255]), // Green - both
                (true, false) => Rgba([59, 130, 246, 255]), // Blue - C only
                (false, true) => Rgba([249, 115, 22, 255]), // Orange - Rust only
                (false, false) => Rgba([255, 255, 255, 255]), // White - neither
            };

            diff.put_pixel(x, y + legend_height, color);
        }
    }

    // Encode as PNG
    let mut buf = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut buf);
    diff.write_with_encoder(encoder).ok()?;
    Some(buf)
}
