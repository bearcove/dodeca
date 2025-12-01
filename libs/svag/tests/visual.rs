//! Visual regression tests for savage.
//!
//! These tests render SVGs before and after minification using headless Chrome,
//! then compare them using SSIM to ensure visual fidelity.
//!
//! Test outputs are saved to `test_output/` (gitignored).

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use image::RgbImage;

use savage::minify;

const TEST_OUTPUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test_output");

/// Minimum acceptable SSIM score (99.9% similarity)
const MIN_SSIM: f64 = 0.999;

/// Render an SVG string to a PNG image using headless Chrome CLI.
fn render_svg(svg: &str, width: u32, height: u32) -> RgbImage {
    // Write SVG to temp file
    let mut svg_file = tempfile::Builder::new()
        .suffix(".svg")
        .tempfile()
        .unwrap();
    svg_file.write_all(svg.as_bytes()).unwrap();
    let svg_path = svg_file.path();

    // Create temp file for output PNG (needs .png extension for image crate)
    let png_file = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .unwrap();
    let png_path = png_file.path().to_string_lossy().to_string();

    // Run Chrome headless to capture screenshot
    let output = Command::new("/usr/bin/chromium")
        .args([
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            "--disable-software-rasterizer",
            &format!("--window-size={},{}", width, height),
            &format!("--screenshot={}", png_path),
            &format!("file://{}", svg_path.display()),
        ])
        .output()
        .expect("Failed to run chromium");

    if !output.status.success() {
        panic!(
            "Chrome failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Load the screenshot
    let img = image::open(&png_path).expect("Failed to open screenshot");
    img.to_rgb8()
}

/// Compute SSIM between two images using the ssim crate.
fn compute_ssim(img1: &RgbImage, img2: &RgbImage) -> f64 {
    let (w1, h1) = img1.dimensions();
    let (w2, h2) = img2.dimensions();

    if w1 != w2 || h1 != h2 {
        return 0.0;
    }

    // Convert to raw RGB bytes
    let bytes1: Vec<u8> = img1.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let bytes2: Vec<u8> = img2.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();

    let ssim_img1 = ssim::Image::from_rgb_image(&bytes1);
    let ssim_img2 = ssim::Image::from_rgb_image(&bytes2);

    ssim_img1.ssim(&ssim_img2) as f64
}

/// Test that minifying an SVG preserves visual appearance.
fn test_visual_fidelity(svg: &str, name: &str) {
    let minified = minify(svg).expect("Failed to minify SVG");

    // Ensure output directory exists
    let output_dir = Path::new(TEST_OUTPUT_DIR);
    fs::create_dir_all(output_dir).unwrap();

    // Parse original to get dimensions
    let (width, height) = extract_dimensions(svg).unwrap_or((256, 256));

    let original_img = render_svg(svg, width, height);
    let minified_img = render_svg(&minified, width, height);

    // Save images for inspection
    let original_path = output_dir.join(format!("{}_original.png", name));
    let minified_path = output_dir.join(format!("{}_minified.png", name));
    original_img.save(&original_path).unwrap();
    minified_img.save(&minified_path).unwrap();

    // Also save the SVGs
    let original_svg_path = output_dir.join(format!("{}_original.svg", name));
    let minified_svg_path = output_dir.join(format!("{}_minified.svg", name));
    fs::write(&original_svg_path, svg).unwrap();
    fs::write(&minified_svg_path, &minified).unwrap();

    let ssim_score = compute_ssim(&original_img, &minified_img);

    println!(
        "{}: SSIM = {:.6} ({}) - saved to test_output/",
        name,
        ssim_score,
        if ssim_score >= MIN_SSIM { "PASS" } else { "FAIL" }
    );

    assert!(
        ssim_score >= MIN_SSIM,
        "{}: SSIM {} is below threshold {} -- see test_output/{}_*.png",
        name,
        ssim_score,
        MIN_SSIM,
        name
    );
}

/// Extract width and height from SVG.
fn extract_dimensions(svg: &str) -> Option<(u32, u32)> {
    let width = extract_attr(svg, "width")?;
    let height = extract_attr(svg, "height")?;

    let w: f64 = width
        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
        .parse()
        .ok()?;
    let h: f64 = height
        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
        .parse()
        .ok()?;

    Some((w.max(100.0) as u32, h.max(100.0) as u32))
}

fn extract_attr(svg: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = svg.find(&pattern)? + pattern.len();
    let end = svg[start..].find('"')? + start;
    Some(svg[start..end].to_string())
}

#[test]
fn test_visual_regression() {
    // === Simple shapes ===
    let simple_tests = [
        (
            "simple_rect",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <rect x="10" y="10" width="80" height="80" fill="red"/>
            </svg>"#,
        ),
        (
            "circle_with_stroke",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <circle cx="50" cy="50" r="40" fill="blue" stroke="black" stroke-width="2"/>
            </svg>"#,
        ),
        (
            "path_triangle",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <path d="M 50 10 L 90 90 L 10 90 Z" fill="#00ff00"/>
            </svg>"##,
        ),
    ];

    println!("\n=== Simple Shapes ===");
    for (name, svg) in simple_tests {
        test_visual_fidelity(svg, name);
    }

    // === Complex paths ===
    println!("\n=== Complex Paths ===");

    let bezier_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <path d="M 10 80 C 40 10, 65 10, 95 80 S 150 150, 180 80"
              fill="none" stroke="black" stroke-width="2"/>
    </svg>"#;
    test_visual_fidelity(bezier_svg, "cubic_bezier");

    let arc_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <path d="M 50 100 A 50 50 0 1 1 150 100 A 50 50 0 1 1 50 100"
              fill="purple"/>
    </svg>"#;
    test_visual_fidelity(arc_svg, "arcs");

    println!("\n=== All visual tests passed! ===\n");
}
