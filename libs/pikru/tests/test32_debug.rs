//! Debug test for test32 vertex positions
//!
//! Run with: cargo test test32_debug -- --nocapture

use pikru_compare::{extract_svg, parse_svg, run_c_pikchr};
use camino::Utf8Path;

const C_PIKCHR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/pikchr-c/pikchr");
const TEST32: &str = include_str!("../vendor/pikchr-c/tests/test32.pikchr");

/// Extract circle elements from SVG as (cx, cy, r, fill_color)
fn extract_circles(svg_str: &str) -> Vec<(f64, f64, f64, String)> {
    let mut circles = Vec::new();

    // Find each <circle .../> element
    let mut rest = svg_str;
    while let Some(start) = rest.find("<circle") {
        rest = &rest[start..];
        let end = rest.find("/>").unwrap_or(rest.len()) + 2;
        let circle_elem = &rest[..end];

        let cx = extract_attr(circle_elem, "cx").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let cy = extract_attr(circle_elem, "cy").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let r = extract_attr(circle_elem, "r").and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let fill = extract_style_prop(circle_elem, "fill").unwrap_or_else(|| "unknown".to_string());

        circles.push((cx, cy, r, fill));
        rest = &rest[end..];
    }
    circles
}

fn extract_attr(line: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = line.find(&pattern)? + pattern.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_style_prop(line: &str, prop: &str) -> Option<String> {
    // Try style="fill: X" format
    let style_start = line.find("style=\"")?;
    let style_content = &line[style_start + 7..];
    let style_end = style_content.find('"')?;
    let style = &style_content[..style_end];

    // Find prop in style
    let prop_pattern = format!("{}:", prop);
    if let Some(idx) = style.find(&prop_pattern) {
        let rest = &style[idx + prop_pattern.len()..];
        let rest = rest.trim_start();
        // Find end (semicolon or end of string)
        let end = rest.find(';').unwrap_or(rest.len());
        return Some(rest[..end].trim().to_string());
    }

    // Also try fill="X" attribute format
    if prop == "fill" {
        if let Some(fill) = extract_attr(line, "fill") {
            return Some(fill);
        }
    }

    None
}

fn extract_viewbox(svg_str: &str) -> Option<(f64, f64, f64, f64)> {
    let start = svg_str.find("viewBox=\"")? + 9;
    let rest = &svg_str[start..];
    let end = rest.find('"')?;
    let parts: Vec<f64> = rest[..end]
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

#[test]
fn debug_test32_circles() {
    // Get C output
    let c_output = run_c_pikchr(Utf8Path::new(C_PIKCHR), TEST32);
    let c_svg = extract_svg(&c_output).expect("C should produce SVG");

    // Get Rust output
    let rust_output = pikru::pikchr(TEST32).expect("Rust should produce SVG");
    let rust_svg = extract_svg(&rust_output).expect("Rust should produce SVG");

    // Compare viewboxes
    let c_vb = extract_viewbox(c_svg);
    let rust_vb = extract_viewbox(&rust_svg);

    println!("\n=== VIEWBOX COMPARISON ===");
    println!("C:    {:?}", c_vb);
    println!("Rust: {:?}", rust_vb);

    // Compare circles
    let c_circles = extract_circles(c_svg);
    let rust_circles = extract_circles(&rust_svg);

    println!("\n=== CIRCLE COMPARISON ===");
    println!("C has {} circles, Rust has {} circles", c_circles.len(), rust_circles.len());

    println!("\n--- C Circles ---");
    for (i, (cx, cy, r, fill)) in c_circles.iter().enumerate() {
        println!("  {}: cx={:.3} cy={:.3} r={:.3} fill={}", i, cx, cy, r, fill);
    }

    println!("\n--- Rust Circles ---");
    for (i, (cx, cy, r, fill)) in rust_circles.iter().enumerate() {
        println!("  {}: cx={:.3} cy={:.3} r={:.3} fill={}", i, cx, cy, r, fill);
    }

    // Show differences
    println!("\n=== DIFFERENCES ===");
    let min_len = c_circles.len().min(rust_circles.len());
    for i in 0..min_len {
        let (c_cx, c_cy, c_r, c_fill) = &c_circles[i];
        let (r_cx, r_cy, r_r, r_fill) = &rust_circles[i];

        let cx_diff = (c_cx - r_cx).abs();
        let cy_diff = (c_cy - r_cy).abs();
        let r_diff = (c_r - r_r).abs();

        if cx_diff > 0.01 || cy_diff > 0.01 || r_diff > 0.01 || c_fill != r_fill {
            println!("Circle {}:", i);
            if cx_diff > 0.01 { println!("  cx: C={:.3} Rust={:.3} diff={:.3}", c_cx, r_cx, cx_diff); }
            if cy_diff > 0.01 { println!("  cy: C={:.3} Rust={:.3} diff={:.3}", c_cy, r_cy, cy_diff); }
            if r_diff > 0.01 { println!("  r:  C={:.3} Rust={:.3} diff={:.3}", c_r, r_r, r_diff); }
            if c_fill != r_fill { println!("  fill: C={} Rust={}", c_fill, r_fill); }
        }
    }
}

#[test]
fn debug_test32_vertex_positions() {
    // The test file places dots at specific vertices of splines
    // Let's trace what positions Rust computes

    let source = r#"
spline right then up then left then down ->
"#;

    let result = pikru::pikchr(source).expect("should parse");
    println!("\n=== Simple spline SVG ===");
    println!("{}", result);
}

#[test]
fn debug_rad_150_percent() {
    // Test the "rad 150%" with "same" bug
    // The handoff says radii are decreasing instead of staying constant

    let source = r#"
dot rad 150%
dot same
dot same
dot same
"#;

    // Get C output for comparison
    let c_output = run_c_pikchr(Utf8Path::new(C_PIKCHR), source);
    let c_svg = extract_svg(&c_output).expect("C should produce SVG");
    let c_circles = extract_circles(c_svg);

    let rust_output = pikru::pikchr(source).expect("should parse");
    let rust_circles = extract_circles(&rust_output);

    println!("\n=== rad 150% with same ===");

    println!("\n--- C circles ---");
    for (i, (cx, cy, r, _)) in c_circles.iter().enumerate() {
        println!("  dot {}: cx={:.3} cy={:.3} r={:.6}", i, cx, cy, r);
    }

    println!("\n--- Rust circles ---");
    for (i, (cx, cy, r, _)) in rust_circles.iter().enumerate() {
        println!("  dot {}: cx={:.3} cy={:.3} r={:.6}", i, cx, cy, r);
    }

    // All radii should be equal (the "same" should copy the radius)
    if rust_circles.len() >= 2 {
        let first_r = rust_circles[0].2;
        for (i, (_, _, r, _)) in rust_circles.iter().enumerate().skip(1) {
            assert!(
                (first_r - r).abs() < 0.001,
                "Dot {} radius {:.6} should equal first dot radius {:.6}",
                i, r, first_r
            );
        }
    }
}
