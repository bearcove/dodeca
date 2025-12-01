//! Visual regression tests for savage.
//!
//! These tests render SVGs before and after minification using headless Chrome,
//! then compare them using SSIM to ensure visual fidelity.
//!
//! Test outputs are saved to `test_output/` (gitignored).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use futures::StreamExt;
use image::RgbImage;
use tempfile::TempDir;

use savage::minify;

const TEST_OUTPUT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test_output");

/// Minimum acceptable SSIM score (99.9% similarity)
const MIN_SSIM: f64 = 0.999;

/// Compute SSIM between two images using the ssim crate.
fn compute_ssim(img1: &RgbImage, img2: &RgbImage) -> f64 {
    let (w1, h1) = img1.dimensions();
    let (w2, h2) = img2.dimensions();

    if w1 != w2 || h1 != h2 {
        return 0.0;
    }

    let bytes1: Vec<u8> = img1.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let bytes2: Vec<u8> = img2.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();

    let ssim_img1 = ssim::Image::from_rgb_image(&bytes1);
    let ssim_img2 = ssim::Image::from_rgb_image(&bytes2);

    ssim_img1.ssim(&ssim_img2) as f64
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

/// Render an SVG string to a PNG image using the browser.
async fn render_svg(browser: &Browser, svg: &str, width: u32, height: u32) -> RgbImage {
    let page = browser.new_page("about:blank").await.unwrap();

    // Create a data URL from the SVG
    let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, svg.as_bytes());
    let data_url = format!("data:image/svg+xml;base64,{}", encoded);

    // Set viewport size
    let _ = page
        .execute(
            chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder()
                .width(width as i64)
                .height(height as i64)
                .device_scale_factor(1.0)
                .mobile(false)
                .build()
                .unwrap(),
        )
        .await;

    // Navigate to the SVG
    page.goto(&data_url).await.unwrap();

    // Wait for rendering
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Take screenshot
    let screenshot = page
        .screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .build(),
        )
        .await
        .unwrap();

    // Decode PNG to image
    let img = image::load_from_memory(&screenshot).unwrap();
    img.to_rgb8()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_visual_regression() {
    // Create a unique temp directory for this browser instance
    let temp_dir = TempDir::new().unwrap();
    let user_data_dir = temp_dir.path().to_string_lossy().to_string();

    // Let chromiumoxide find the browser automatically (no chrome_executable specified)
    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg(format!("--user-data-dir={}", user_data_dir))
            .build()
            .unwrap(),
    )
    .await
    .expect("Failed to launch browser - is Chrome/Chromium installed?");

    let browser = Arc::new(browser);

    // Spawn handler task - must keep processing events
    let handle = tokio::spawn(async move {
        loop {
            match handler.next().await {
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    eprintln!("Browser handler error: {:?}", e);
                }
                None => break,
            }
        }
    });

    // Give the browser a moment to initialize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Ensure output directory exists
    let output_dir = Path::new(TEST_OUTPUT_DIR);
    fs::create_dir_all(output_dir).unwrap();

    // Test cases
    let test_cases = [
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
        (
            "cubic_bezier",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
                <path d="M 10 80 C 40 10, 65 10, 95 80 S 150 150, 180 80"
                      fill="none" stroke="black" stroke-width="2"/>
            </svg>"#,
        ),
        (
            "arcs",
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
                <path d="M 50 100 A 50 50 0 1 1 150 100 A 50 50 0 1 1 50 100"
                      fill="purple"/>
            </svg>"#,
        ),
    ];

    println!("\n=== Visual Regression Tests ===");

    for (name, svg) in test_cases {
        let minified = minify(svg).expect("Failed to minify SVG");
        let (width, height) = extract_dimensions(svg).unwrap_or((256, 256));

        // Render original
        let original_img = render_svg(&browser, svg, width, height).await;
        // Render minified
        let minified_img = render_svg(&browser, &minified, width, height).await;

        // Save for inspection
        let original_path = output_dir.join(format!("{}_original.png", name));
        let minified_path = output_dir.join(format!("{}_minified.png", name));
        original_img.save(&original_path).unwrap();
        minified_img.save(&minified_path).unwrap();

        // Save SVGs too
        fs::write(output_dir.join(format!("{}_original.svg", name)), svg).unwrap();
        fs::write(output_dir.join(format!("{}_minified.svg", name)), &minified).unwrap();

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

    println!("\n=== All visual tests passed! ===\n");

    // Cleanup
    drop(browser);
    handle.abort();
}
