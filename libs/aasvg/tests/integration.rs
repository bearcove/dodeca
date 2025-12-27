//! Integration tests for aasvg.
//!
//! These tests verify that complete diagrams render correctly.

use aasvg::{render, render_with_options, RenderOptions};

const FIXTURES_DIR: &str = "tests/fixtures";

fn load_fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/{}", FIXTURES_DIR, name))
        .expect("Failed to read fixture file")
}

// ============================================================================
// Basic rendering tests
// ============================================================================

#[test]
fn test_render_simple_box() {
    let diagram = "+--+\n|  |\n+--+";
    let svg = render(diagram);

    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("<path")); // Lines
    assert!(svg.contains("var(--aasvg-stroke)")); // CSS variable usage
}

#[test]
fn test_render_horizontal_arrow() {
    let svg = render("--->");
    assert!(svg.contains("<path")); // Line
    assert!(svg.contains("<polygon")); // Arrow head
    assert!(svg.contains("var(--aasvg-fill)")); // Arrow uses fill variable
}

#[test]
fn test_render_vertical_arrow() {
    let svg = render("|\n|\nv");
    assert!(svg.contains("<path"));
    assert!(svg.contains("<polygon"));
}

#[test]
fn test_render_text() {
    let svg = render("Hello World");
    assert!(svg.contains("<text"));
    assert!(svg.contains("Hello"));
    assert!(svg.contains("World"));
    assert!(svg.contains("var(--aasvg-text)")); // Text uses CSS variable
}

#[test]
fn test_render_mixed_diagram() {
    let diagram = r#"
+--------+
| Hello! |---->
+--------+
"#;
    let svg = render(diagram);

    // Should have paths (lines)
    assert!(svg.contains("<path"));
    // Should have text
    assert!(svg.contains("<text"));
    // Should have arrow
    assert!(svg.contains("<polygon"));
}

// ============================================================================
// CSS variable tests
// ============================================================================

#[test]
fn test_css_variables_present() {
    let svg = render("-");

    // Light mode variables
    assert!(svg.contains("--aasvg-stroke"));
    assert!(svg.contains("--aasvg-fill"));
    assert!(svg.contains("--aasvg-bg"));
    assert!(svg.contains("--aasvg-text"));

    // Dark mode media query
    assert!(svg.contains("prefers-color-scheme: dark"));
}

#[test]
fn test_stroke_uses_variable() {
    let svg = render("---");
    assert!(svg.contains(r#"stroke="var(--aasvg-stroke)"#));
}

#[test]
fn test_fill_uses_variable() {
    let svg = render("-->");
    assert!(svg.contains(r#"fill="var(--aasvg-fill)"#));
}

#[test]
fn test_backdrop_uses_variable() {
    let options = RenderOptions::new().with_backdrop(true);
    let svg = render_with_options("-", &options);
    assert!(svg.contains(r#"fill="var(--aasvg-bg)"#));
}

// ============================================================================
// Options tests
// ============================================================================

#[test]
fn test_backdrop_option() {
    let with_backdrop = render_with_options("-", &RenderOptions::new().with_backdrop(true));
    let _without_backdrop = render_with_options("-", &RenderOptions::new().with_backdrop(false));

    assert!(with_backdrop.contains("<rect"));
    // The without_backdrop might still have rect for other reasons, so just verify the with case
}

#[test]
fn test_disable_text_option() {
    let with_text = render("Hello");
    let without_text =
        render_with_options("Hello", &RenderOptions::new().with_disable_text(true));

    assert!(with_text.contains("Hello"));
    assert!(!without_text.contains("Hello"));
}

// ============================================================================
// Fixture tests
// ============================================================================

#[test]
fn test_fixture_boxes() {
    let input = load_fixture("boxes.txt");
    let svg = render(&input);

    // Basic structure
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));

    // Should contain paths for the box lines
    assert!(svg.contains("<path"));

    // Should have CSS variables
    assert!(svg.contains("--aasvg-stroke"));
}

#[test]
fn test_fixture_arrows() {
    let input = load_fixture("arrows.txt");
    let svg = render(&input);

    // Should have arrow heads
    assert!(svg.contains("<polygon"));

    // Should have paths for lines
    assert!(svg.contains("<path"));
}

#[test]
fn test_fixture_curves() {
    let input = load_fixture("curves.txt");
    let svg = render(&input);

    // Should render something
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
}

#[test]
fn test_fixture_points() {
    let input = load_fixture("points.txt");
    let svg = render(&input);

    // Should have circles for points
    assert!(svg.contains("<circle"));
}

#[test]
fn test_fixture_example() {
    let input = load_fixture("example.txt");
    let svg = render(&input);

    // The example file is complex and should produce a rich SVG
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));

    // Should have all element types
    assert!(svg.contains("<path")); // Lines
    assert!(svg.contains("<polygon")); // Arrows
    assert!(svg.contains("<text")); // Text

    // Should have CSS variables
    assert!(svg.contains("--aasvg-stroke"));
    assert!(svg.contains("prefers-color-scheme: dark"));
}

// ============================================================================
// Edge cases
// ============================================================================

#[test]
fn test_empty_input() {
    let svg = render("");
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
}

#[test]
fn test_whitespace_only() {
    let svg = render("   \n   \n   ");
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>"));
}

#[test]
fn test_single_character() {
    let svg = render("X");
    assert!(svg.contains("<text"));
    assert!(svg.contains("X"));
}

#[test]
fn test_unicode_text() {
    let svg = render("日本語");
    assert!(svg.contains("<text"));
    assert!(svg.contains("日本語"));
}

#[test]
fn test_special_characters() {
    let svg = render("<>&\"");
    // Special characters should be escaped
    assert!(svg.contains("&lt;"));
    assert!(svg.contains("&gt;"));
    assert!(svg.contains("&amp;"));
}

// ============================================================================
// Line type tests
// ============================================================================

#[test]
fn test_horizontal_line() {
    let svg = render("-----");
    assert!(svg.contains("<path"));
}

#[test]
fn test_vertical_line() {
    let svg = render("|\n|\n|\n|");
    assert!(svg.contains("<path"));
}

#[test]
fn test_diagonal_line() {
    let svg = render("\\\n \\");
    assert!(svg.contains("<path"));
}

#[test]
fn test_forward_diagonal() {
    let svg = render(" /\n/");
    assert!(svg.contains("<path"));
}

#[test]
fn test_double_line() {
    let svg = render("=====");
    assert!(svg.contains("<path"));
}

#[test]
fn test_squiggle_line() {
    let svg = render("~~~~~");
    assert!(svg.contains("<path"));
    // Squiggle should have Q (quadratic curve) commands
    assert!(svg.contains(" Q "));
}
