//! Corpus tests - run minification on all SVGs in the corpus directory.
//! These tests verify that minification produces valid output without Chrome.

use std::fs;
use std::path::Path;

use svag::{minify, parse_svg};

/// Test that all corpus SVGs can be parsed and minified without errors.
#[test]
fn test_corpus_minification() {
    let corpus_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");

    if !corpus_dir.exists() {
        println!("Corpus directory not found, skipping");
        return;
    }

    let mut total = 0;
    let mut total_original = 0usize;
    let mut total_minified = 0usize;

    for entry in fs::read_dir(&corpus_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().map(|e| e == "svg").unwrap_or(false) {
            let name = path.file_name().unwrap().to_string_lossy();
            let content = fs::read_to_string(&path).unwrap();
            let original_size = content.len();

            // Test parsing
            let doc = parse_svg(&content).unwrap_or_else(|_| panic!("Failed to parse {}", name));

            // Verify root is an SVG element
            assert!(doc.root.is("svg"), "{}: Root element is not <svg>", name);

            // Test minification
            let minified = minify(&content).unwrap_or_else(|_| panic!("Failed to minify {}", name));
            let minified_size = minified.len();

            // Verify minified output is valid SVG
            let _ = parse_svg(&minified)
                .unwrap_or_else(|_| panic!("Failed to parse minified output of {}", name));

            // Calculate savings
            let savings = if original_size > 0 {
                ((original_size - minified_size) as f64 / original_size as f64) * 100.0
            } else {
                0.0
            };

            println!(
                "{}: {} -> {} bytes ({:.1}% smaller)",
                name, original_size, minified_size, savings
            );

            total += 1;
            total_original += original_size;
            total_minified += minified_size;
        }
    }

    if total > 0 {
        let total_savings =
            ((total_original - total_minified) as f64 / total_original as f64) * 100.0;
        println!(
            "\nTotal: {} files, {} -> {} bytes ({:.1}% smaller)",
            total, total_original, total_minified, total_savings
        );
    }
}

/// Test specific optimization behaviors.
#[test]
fn test_inkscape_cleanup() {
    let inkscape_svg = r#"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg"
     xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
     xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
     inkscape:version="1.0"
     sodipodi:docname="test.svg">
  <sodipodi:namedview inkscape:zoom="1"/>
  <g inkscape:label="Layer 1">
    <rect x="0" y="0" width="100" height="100"/>
  </g>
</svg>"#;

    let minified = minify(inkscape_svg).unwrap();

    // Should not contain inkscape or sodipodi
    assert!(
        !minified.contains("inkscape:"),
        "inkscape namespace not removed"
    );
    assert!(
        !minified.contains("sodipodi:"),
        "sodipodi namespace not removed"
    );
    assert!(
        !minified.contains("sodipodi:namedview"),
        "sodipodi:namedview not removed"
    );

    // Should still be valid SVG
    let doc = parse_svg(&minified).unwrap();
    assert!(doc.root.is("svg"));
}

/// Test that path precision is reduced.
#[test]
fn test_path_precision() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
        <path d="M 10.123456789 20.987654321 L 30.111111111 40.222222222"/>
    </svg>"#;

    let minified = minify(svg).unwrap();

    // Should not contain high-precision numbers
    assert!(
        !minified.contains("123456789"),
        "High precision not reduced"
    );
    assert!(
        !minified.contains("987654321"),
        "High precision not reduced"
    );

    // Should still parse correctly
    let _ = parse_svg(&minified).unwrap();
}

/// Test color minification.
#[test]
fn test_color_minification() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg">
        <rect fill="#ff0000"/>
        <rect fill="#ffffff"/>
        <rect fill="#aabbcc"/>
    </svg>"##;

    let minified = minify(svg).unwrap();

    // #ff0000 should become red
    assert!(minified.contains("red"), "red color not shortened");
    // #ffffff should become #fff
    assert!(minified.contains("#fff"), "#ffffff not shortened to #fff");
    // #aabbcc should become #abc
    assert!(minified.contains("#abc"), "#aabbcc not shortened to #abc");
}

/// Test that default values are removed.
#[test]
fn test_default_removal() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1">
        <rect fill-opacity="1" stroke-opacity="1" opacity="1"/>
    </svg>"#;

    let minified = minify(svg).unwrap();

    // version="1.1" should be removed
    assert!(
        !minified.contains("version="),
        "version attribute not removed"
    );
    // fill-opacity="1" should be removed
    assert!(
        !minified.contains("fill-opacity"),
        "fill-opacity not removed"
    );
    // stroke-opacity="1" should be removed
    assert!(
        !minified.contains("stroke-opacity"),
        "stroke-opacity not removed"
    );
    // opacity="1" should be removed
    assert!(!minified.contains("opacity="), "opacity not removed");
}
