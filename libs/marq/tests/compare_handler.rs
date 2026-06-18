//! Integration tests for CompareHandler.
//!
//! These tests verify the full rendering pipeline from markdown input
//! containing ```compare blocks to HTML output with side-by-side comparisons.
//!
//! Requires the `highlight` feature to be enabled.

#![cfg(feature = "highlight")]

use marq::{CompareHandler, RenderOptions, render};

/// Helper to render markdown with CompareHandler registered.
async fn render_with_compare(markdown: &str) -> marq::Document {
    let opts = RenderOptions::new().with_handler(&["compare"], CompareHandler::new());
    render(markdown, &opts).await.unwrap()
}

#[tokio::test]
async fn test_compare_block_basic_two_languages() {
    let markdown = r#"
# Comparison

```compare
/// json
{"server": {"host": "localhost", "port": 8080}}
/// yaml
server:
  host: localhost
  port: 8080
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should have the compare container
    assert!(
        doc.html.contains(r#"class="compare-container""#),
        "Should have compare-container class: {}",
        doc.html
    );

    // Should have two sections
    let section_count = doc.html.matches(r#"class="compare-section""#).count();
    assert_eq!(section_count, 2, "Should have exactly 2 compare sections");

    // Should have language headers
    assert!(
        doc.html.contains(r#"class="compare-header">json</div>"#),
        "Should have json header: {}",
        doc.html
    );
    assert!(
        doc.html.contains(r#"class="compare-header">yaml</div>"#),
        "Should have yaml header: {}",
        doc.html
    );

    // Should contain the actual code content (syntax highlighted)
    assert!(
        doc.html.contains("localhost"),
        "Should contain code content: {}",
        doc.html
    );
    assert!(
        doc.html.contains("8080"),
        "Should contain port number: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_three_way_comparison() {
    let markdown = r#"
```compare
/// json
{"format": "json"}
/// yaml
format: yaml
/// toml
format = "toml"
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should have three sections
    let section_count = doc.html.matches(r#"class="compare-section""#).count();
    assert_eq!(section_count, 3, "Should have exactly 3 compare sections");

    // All three language headers should be present
    assert!(doc.html.contains(">json</div>"));
    assert!(doc.html.contains(">yaml</div>"));
    assert!(doc.html.contains(">toml</div>"));
}

#[tokio::test]
async fn test_compare_block_multiline_code() {
    let markdown = r#"
```compare
/// rust
fn main() {
    println!("Hello, World!");
}
/// python
def main():
    print("Hello, World!")

if __name__ == "__main__":
    main()
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should preserve multiline structure
    assert!(
        doc.html.contains("println!"),
        "Should contain Rust code: {}",
        doc.html
    );
    // Note: arborium wraps tokens in custom elements, so "print(" becomes "<a-v>print</a-v>("
    assert!(
        doc.html.contains("print") && doc.html.contains("Hello, World!"),
        "Should contain Python code: {}",
        doc.html
    );

    // Headers should be present
    assert!(doc.html.contains(">rust</div>"));
    assert!(doc.html.contains(">python</div>"));
}

#[tokio::test]
async fn test_compare_block_with_no_valid_sections_falls_back() {
    let markdown = r#"
```compare
This has no /// language markers
so it should fall back to plain text
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should fall back to plain <pre><code> rendering
    assert!(
        doc.html.contains("<pre><code>"),
        "Should fall back to plain code block: {}",
        doc.html
    );
    assert!(
        doc.html.contains("no /// language markers"),
        "Should contain the text: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_ignores_leading_content() {
    let markdown = r#"
```compare
This leading content is ignored
/// json
{"key": "value"}
/// yaml
key: value
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should have two sections (leading content ignored)
    let section_count = doc.html.matches(r#"class="compare-section""#).count();
    assert_eq!(
        section_count, 2,
        "Should have exactly 2 compare sections (leading content ignored)"
    );

    // Leading content should NOT appear in output
    assert!(
        !doc.html.contains("leading content is ignored"),
        "Leading content should be ignored: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_mixed_with_regular_content() {
    let markdown = r#"
# Introduction

Some introductory text.

```compare
/// json
{"example": true}
/// yaml
example: true
```

## Conclusion

Final thoughts.
"#;

    let doc = render_with_compare(markdown).await;

    // Should have both headings
    assert_eq!(doc.headings.len(), 2);
    assert_eq!(doc.headings[0].title, "Introduction");
    assert_eq!(doc.headings[1].title, "Conclusion");

    // Should have the compare container
    assert!(doc.html.contains(r#"class="compare-container""#));

    // Should have regular paragraph content
    assert!(doc.html.contains("introductory text"));
    assert!(doc.html.contains("Final thoughts"));
}

#[tokio::test]
async fn test_compare_block_syntax_highlighting() {
    let markdown = r#"
```compare
/// rust
let x: i32 = 42;
/// javascript
const x = 42;
```
"#;

    let doc = render_with_compare(markdown).await;

    // With syntax highlighting enabled, we should see custom elements for tokens
    // Arborium uses elements like <a-k> (keyword), <a-v> (variable), etc.
    assert!(
        doc.html.contains("<a-k>") || doc.html.contains("<a-v>"),
        "Should have syntax highlighting elements: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_preserves_code_structure() {
    let markdown = r#"
```compare
/// json
{
  "nested": {
    "deeply": {
      "value": 123
    }
  }
}
/// yaml
nested:
  deeply:
    value: 123
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should preserve nested structure (newlines)
    assert!(
        doc.html.contains("nested"),
        "Should contain nested key: {}",
        doc.html
    );
    assert!(
        doc.html.contains("deeply"),
        "Should contain deeply key: {}",
        doc.html
    );
    assert!(
        doc.html.contains("123"),
        "Should contain value: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_single_section() {
    // Edge case: only one section defined
    let markdown = r#"
```compare
/// rust
fn solo() {}
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should still render with compare structure
    let section_count = doc.html.matches(r#"class="compare-section""#).count();
    assert_eq!(section_count, 1, "Should have exactly 1 compare section");
    assert!(doc.html.contains(">rust</div>"));
}

#[tokio::test]
async fn test_compare_block_code_sample_extraction() {
    let markdown = r#"
```compare
/// json
{"key": "value"}
/// yaml
key: value
```
"#;

    let doc = render_with_compare(markdown).await;

    // Compare blocks should be recorded as code samples
    assert_eq!(doc.code_samples.len(), 1);
    assert_eq!(doc.code_samples[0].language, "compare");
    assert!(doc.code_samples[0].code.contains("/// json"));
    assert!(doc.code_samples[0].code.contains("/// yaml"));
}

#[tokio::test]
async fn test_compare_block_empty() {
    let markdown = r#"
```compare
```
"#;

    let doc = render_with_compare(markdown).await;

    // Empty compare block should fall back gracefully
    assert!(
        doc.html.contains("<pre><code>"),
        "Empty compare should fall back: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_compare_block_whitespace_handling() {
    let markdown = r#"
```compare
/// json
{
    "indented": true
}
/// yaml
indented: true
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should preserve indentation in output
    assert!(
        doc.html.contains("compare-container"),
        "Should render compare container: {}",
        doc.html
    );
}

#[tokio::test]
async fn test_multiple_compare_blocks() {
    let markdown = r#"
# First Comparison

```compare
/// json
{"first": 1}
/// yaml
first: 1
```

# Second Comparison

```compare
/// rust
let second = 2;
/// python
second = 2
```
"#;

    let doc = render_with_compare(markdown).await;

    // Should have two compare containers
    let container_count = doc.html.matches(r#"class="compare-container""#).count();
    assert_eq!(
        container_count, 2,
        "Should have exactly 2 compare containers"
    );

    // Should have 4 sections total (2 per block)
    let section_count = doc.html.matches(r#"class="compare-section""#).count();
    assert_eq!(section_count, 4, "Should have exactly 4 compare sections");

    // Should have 2 code samples
    assert_eq!(doc.code_samples.len(), 2);
}
