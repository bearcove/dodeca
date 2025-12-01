//! Visual regression tests for savage.
//!
//! These tests render SVGs before and after minification using headless Chrome,
//! then compare them using SSIM to ensure visual fidelity.

use std::sync::Arc;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use futures::StreamExt;
use image::RgbImage;

use savage::minify;

/// Minimum acceptable SSIM score (99.9% similarity)
const MIN_SSIM: f64 = 0.999;

/// Render an SVG string to a PNG image using headless Chrome.
async fn render_svg(browser: &Browser, svg: &str, width: u32, height: u32) -> RgbImage {
    let page = browser.new_page("about:blank").await.unwrap();

    // Create a data URL from the SVG
    let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, svg.as_bytes());
    let data_url = format!("data:image/svg+xml;base64,{}", encoded);

    // Set up the page with proper viewport via emulation
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

    // Wait a bit for rendering
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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

/// Compute SSIM (Structural Similarity Index) between two images.
/// Returns a value between 0 and 1, where 1 means identical.
fn compute_ssim(img1: &RgbImage, img2: &RgbImage) -> f64 {
    assert_eq!(img1.dimensions(), img2.dimensions());

    let (width, height) = img1.dimensions();
    let n = (width * height) as f64;

    // Constants for SSIM
    let c1 = (0.01 * 255.0_f64).powi(2);
    let c2 = (0.03 * 255.0_f64).powi(2);

    let mut sum1 = 0.0_f64;
    let mut sum2 = 0.0_f64;
    let mut sum1_sq = 0.0_f64;
    let mut sum2_sq = 0.0_f64;
    let mut sum12 = 0.0_f64;

    for y in 0..height {
        for x in 0..width {
            let p1 = img1.get_pixel(x, y);
            let p2 = img2.get_pixel(x, y);

            // Convert to grayscale luminance
            let l1 = 0.299 * p1[0] as f64 + 0.587 * p1[1] as f64 + 0.114 * p1[2] as f64;
            let l2 = 0.299 * p2[0] as f64 + 0.587 * p2[1] as f64 + 0.114 * p2[2] as f64;

            sum1 += l1;
            sum2 += l2;
            sum1_sq += l1 * l1;
            sum2_sq += l2 * l2;
            sum12 += l1 * l2;
        }
    }

    let mu1 = sum1 / n;
    let mu2 = sum2 / n;
    let sigma1_sq = sum1_sq / n - mu1 * mu1;
    let sigma2_sq = sum2_sq / n - mu2 * mu2;
    let sigma12 = sum12 / n - mu1 * mu2;

    let ssim = ((2.0 * mu1 * mu2 + c1) * (2.0 * sigma12 + c2))
        / ((mu1 * mu1 + mu2 * mu2 + c1) * (sigma1_sq + sigma2_sq + c2));

    ssim
}

/// Test that minifying an SVG preserves visual appearance.
async fn test_visual_fidelity(browser: &Browser, svg: &str, name: &str) {
    let minified = minify(svg).expect("Failed to minify SVG");

    // Parse original to get dimensions
    let (width, height) = extract_dimensions(svg).unwrap_or((256, 256));

    let original_img = render_svg(browser, svg, width, height).await;
    let minified_img = render_svg(browser, &minified, width, height).await;

    let ssim = compute_ssim(&original_img, &minified_img);

    println!(
        "{}: SSIM = {:.6} ({})",
        name,
        ssim,
        if ssim >= MIN_SSIM { "PASS" } else { "FAIL" }
    );

    assert!(
        ssim >= MIN_SSIM,
        "{}: SSIM {} is below threshold {}",
        name,
        ssim,
        MIN_SSIM
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

    Some((w as u32, h as u32))
}

fn extract_attr(svg: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = svg.find(&pattern)? + pattern.len();
    let end = svg[start..].find('"')? + start;
    Some(svg[start..end].to_string())
}

#[tokio::test]
async fn test_simple_shapes() {
    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .with_head()
            .build()
            .unwrap(),
    )
    .await
    .unwrap();

    let browser = Arc::new(browser);

    // Spawn handler
    let handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    // Test cases
    let test_svgs = [
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
            "gradient",
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
                <defs>
                    <linearGradient id="grad1">
                        <stop offset="0%" style="stop-color:rgb(255,255,0);stop-opacity:1" />
                        <stop offset="100%" style="stop-color:rgb(255,0,0);stop-opacity:1" />
                    </linearGradient>
                </defs>
                <rect x="0" y="0" width="100" height="100" fill="url(#grad1)"/>
            </svg>"##,
        ),
    ];

    for (name, svg) in test_svgs {
        test_visual_fidelity(&browser, svg, name).await;
    }

    // Cleanup
    drop(browser);
    handle.abort();
}

#[tokio::test]
async fn test_complex_paths() {
    let (browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .with_head()
            .build()
            .unwrap(),
    )
    .await
    .unwrap();

    let browser = Arc::new(browser);

    let handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    // Test with cubic bezier curves
    let bezier_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <path d="M 10 80 C 40 10, 65 10, 95 80 S 150 150, 180 80"
              fill="none" stroke="black" stroke-width="2"/>
    </svg>"#;

    test_visual_fidelity(&browser, bezier_svg, "cubic_bezier").await;

    // Test with arcs
    let arc_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
        <path d="M 50 100 A 50 50 0 1 1 150 100 A 50 50 0 1 1 50 100"
              fill="purple"/>
    </svg>"#;

    test_visual_fidelity(&browser, arc_svg, "arcs").await;

    drop(browser);
    handle.abort();
}
