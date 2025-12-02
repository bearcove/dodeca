//! Integration test for font utilities with real SVG

use svag::{
    Options, extract_font_faces, extract_text_chars, parse_svg, replace_font_url, serialize,
};

const TEST_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200">
  <defs>
    <style>
      @font-face {
        font-family: 'Iosevka';
        src: url('tests/fixtures/Iosevka-Regular.woff2') format('woff2');
        font-weight: normal;
      }
      .code { font-family: 'Iosevka', monospace; }
    </style>
  </defs>
  <text x="20" y="60" class="code">fn main() { }</text>
  <text x="20" y="100" class="code">let x = 42;</text>
</svg>"#;

#[test]
fn test_extract_text_chars_real_svg() {
    let doc = parse_svg(TEST_SVG).unwrap();
    let chars = extract_text_chars(&doc);

    // Should contain characters from both text elements
    assert!(chars.contains(&'f'));
    assert!(chars.contains(&'n'));
    assert!(chars.contains(&'m'));
    assert!(chars.contains(&'a'));
    assert!(chars.contains(&'i'));
    assert!(chars.contains(&'('));
    assert!(chars.contains(&')'));
    assert!(chars.contains(&'{'));
    assert!(chars.contains(&'}'));
    assert!(chars.contains(&'l'));
    assert!(chars.contains(&'e'));
    assert!(chars.contains(&'t'));
    assert!(chars.contains(&'x'));
    assert!(chars.contains(&'='));
    assert!(chars.contains(&'4'));
    assert!(chars.contains(&'2'));
    assert!(chars.contains(&';'));
    assert!(chars.contains(&' '));

    // Should NOT contain characters not in the text
    assert!(!chars.contains(&'Z'));
    assert!(!chars.contains(&'Q'));

    // Verify reasonable count (no duplicates in set)
    assert!(chars.len() < 30); // "fn main() { }let x = 42;" has ~20 unique chars
}

#[test]
fn test_extract_font_faces_real_svg() {
    let doc = parse_svg(TEST_SVG).unwrap();
    let faces = extract_font_faces(&doc);

    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].family, "Iosevka");
    assert_eq!(faces[0].url, "tests/fixtures/Iosevka-Regular.woff2");
    assert_eq!(faces[0].weight, Some("normal".to_string()));
    assert_eq!(faces[0].style, None);
}

#[test]
fn test_replace_and_roundtrip() {
    let mut doc = parse_svg(TEST_SVG).unwrap();

    // Replace font URL with another file path
    replace_font_url(
        &mut doc,
        "tests/fixtures/Iosevka-Regular.woff2",
        "fonts/subset.woff2",
    );

    // Serialize and re-parse
    let serialized = serialize(&doc, &Options::default());
    let doc2 = parse_svg(&serialized).unwrap();

    // Verify the replacement persisted
    let faces = extract_font_faces(&doc2);
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0].url, "fonts/subset.woff2");

    // Text content should be unchanged
    let chars = extract_text_chars(&doc2);
    assert!(chars.contains(&'f'));
    assert!(chars.contains(&'n'));
}
