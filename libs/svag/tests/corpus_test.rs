//! Corpus tests - run minification on all SVGs in the corpus directory.
//! These tests verify that minification produces valid output without Chrome.

use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
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
    let mut passed = 0;
    let mut failed = 0;
    let mut total_original = 0usize;
    let mut total_minified = 0usize;

    for entry in WalkBuilder::new(&corpus_dir).git_ignore(false).build() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "svg") {
            let rel_path = path.strip_prefix(&corpus_dir).unwrap_or(path);
            let content = match fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("  SKIP {}: {}", rel_path.display(), e);
                    continue;
                }
            };
            let original_size = content.len();
            total += 1;

            // Test parsing and minification
            match minify(&content) {
                Ok(minified) => {
                    let minified_size = minified.len();

                    // Verify minified output is valid SVG
                    if let Err(e) = parse_svg(&minified) {
                        eprintln!(
                            "  FAIL {}: minified output invalid: {}",
                            rel_path.display(),
                            e
                        );
                        failed += 1;
                        continue;
                    }

                    passed += 1;
                    total_original += original_size;
                    total_minified += minified_size;
                }
                Err(e) => {
                    eprintln!("  FAIL {}: {}", rel_path.display(), e);
                    failed += 1;
                }
            }
        }
    }

    if total > 0 {
        let total_savings = if total_original > 0 {
            ((total_original - total_minified) as f64 / total_original as f64) * 100.0
        } else {
            0.0
        };
        println!("\nCorpus: {}/{} passed, {} failed", passed, total, failed);
        println!(
            "Size: {} -> {} bytes ({:.1}% smaller)",
            total_original, total_minified, total_savings
        );
    }

    assert_eq!(failed, 0, "{} SVG files failed to minify", failed);
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
