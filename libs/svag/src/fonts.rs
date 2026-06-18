//! Font utilities for SVGs
//!
//! Extract text content and `@font-face` references from SVGs, enabling font
//! subsetting workflows.
//!
//! # Intended Use
//!
//! These functions are building blocks for font subsetting pipelines. They're
//! pure functions designed to be composed with [fontcull](https://crates.io/crates/fontcull)
//! in incremental build systems like [Salsa](https://crates.io/crates/salsa).
//!
//! ## Example: Embedding subsetted fonts
//!
//! ```ignore
//! use svag::{parse_svg, serialize, extract_text_chars, extract_font_faces, replace_font_url};
//! use fontcull::subset_font_to_woff2;
//! use base64::Engine;
//!
//! // 1. Parse the SVG
//! let mut doc = parse_svg(svg_content)?;
//!
//! // 2. Find which characters are used in <text> elements
//! let chars = extract_text_chars(&doc);
//!
//! // 3. Find @font-face declarations
//! for face in extract_font_faces(&doc) {
//!     // 4. Load and subset the font
//!     let font_data = std::fs::read(&face.url)?;
//!     let subsetted = subset_font_to_woff2(&font_data, &chars)?;
//!
//!     // 5. Embed as data URL
//!     let encoded = base64::prelude::BASE64_STANDARD.encode(&subsetted);
//!     let data_url = format!("data:font/woff2;base64,{}", encoded);
//!     replace_font_url(&mut doc, &face.url, &data_url);
//! }
//!
//! // 6. Serialize back to SVG
//! let result = serialize(&doc, &Options::default());
//! ```
//!
//! ## Salsa Integration
//!
//! In a Salsa-based build system, wrap these in tracked queries:
//!
//! ```ignore
//! #[salsa::tracked]
//! fn svg_text_chars(db: &dyn Db, file: StaticFile) -> HashSet<char> {
//!     let doc = svag::parse_svg(file.content(db)).unwrap();
//!     svag::extract_text_chars(&doc)
//! }
//! ```
//!
//! Salsa memoizes based on input changes - svag functions are pure, so caching
//! is handled by the caller.

use crate::{Document, Element, Node};
use std::collections::HashSet;

/// Extract all text content from `<text>` elements in the document
pub fn extract_text_chars(doc: &Document) -> HashSet<char> {
    let mut chars = HashSet::new();

    fn visit(elem: &Element, chars: &mut HashSet<char>) {
        if elem.is("text") || elem.is("tspan") || elem.is("textPath") {
            for child in &elem.children {
                if let Node::Text(t) = child {
                    for c in t.chars() {
                        chars.insert(c);
                    }
                }
            }
        }
        for child in elem.child_elements() {
            visit(child, chars);
        }
    }

    visit(&doc.root, &mut chars);
    chars
}

/// A parsed @font-face reference
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRef {
    pub family: String,
    pub url: String,
    pub weight: Option<String>,
    pub style: Option<String>,
}

/// Extract `@font-face` rules from `<style>` elements
pub fn extract_font_faces(doc: &Document) -> Vec<FontFaceRef> {
    let mut faces = Vec::new();

    fn visit(elem: &Element, faces: &mut Vec<FontFaceRef>) {
        if elem.is("style") {
            for child in &elem.children {
                let css = match child {
                    Node::Text(t) => t.as_str(),
                    Node::CData(t) => t.as_str(),
                    _ => continue,
                };
                faces.extend(parse_font_faces(css));
            }
        }
        for child in elem.child_elements() {
            visit(child, faces);
        }
    }

    visit(&doc.root, &mut faces);
    faces
}

/// Replace a font URL in the document's `<style>` elements
pub fn replace_font_url(doc: &mut Document, old_url: &str, new_url: &str) {
    fn visit(elem: &mut Element, old_url: &str, new_url: &str) {
        if elem.is("style") {
            for child in &mut elem.children {
                let css = match child {
                    Node::Text(t) => t,
                    Node::CData(t) => t,
                    _ => continue,
                };

                // Replace url('old') or url("old") or url(old)
                let patterns = [
                    format!("url('{}')", old_url),
                    format!("url(\"{}\")", old_url),
                    format!("url({})", old_url),
                ];

                for pattern in &patterns {
                    if css.contains(pattern) {
                        *css = css.replace(pattern, &format!("url('{}')", new_url));
                        return;
                    }
                }
            }
        }
        for child in elem.child_elements_mut() {
            visit(child, old_url, new_url);
        }
    }

    visit(&mut doc.root, old_url, new_url);
}

fn parse_font_faces(css: &str) -> Vec<FontFaceRef> {
    let mut faces = Vec::new();
    let mut remaining = css;

    while let Some(start) = remaining.find("@font-face") {
        remaining = &remaining[start + "@font-face".len()..];

        let Some(brace_start) = remaining.find('{') else {
            break;
        };
        remaining = &remaining[brace_start + 1..];

        let mut depth = 1;
        let mut block_end = 0;
        for (i, c) in remaining.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        block_end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if block_end == 0 {
            break;
        }

        let block = &remaining[..block_end];
        remaining = &remaining[block_end + 1..];

        if let Some(face) = parse_font_face_block(block) {
            faces.push(face);
        }
    }

    faces
}

fn parse_font_face_block(block: &str) -> Option<FontFaceRef> {
    let mut family = None;
    let mut url = None;
    let mut weight = None;
    let mut style = None;

    for decl in block.split(';') {
        let decl = decl.trim();
        if let Some(v) = decl.strip_prefix("font-family:") {
            family = Some(parse_value(v));
        } else if let Some(v) = decl.strip_prefix("src:") {
            url = parse_url(v);
        } else if let Some(v) = decl.strip_prefix("font-weight:") {
            weight = Some(v.trim().to_string());
        } else if let Some(v) = decl.strip_prefix("font-style:") {
            style = Some(v.trim().to_string());
        }
    }

    Some(FontFaceRef {
        family: family?,
        url: url?,
        weight,
        style,
    })
}

fn parse_value(v: &str) -> String {
    v.trim()
        .split(',')
        .next()
        .unwrap_or(v)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn parse_url(v: &str) -> Option<String> {
    let start = v.find("url(")? + 4;
    let end = v[start..].find(')')? + start;
    Some(
        v[start..end]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_svg;

    #[test]
    fn test_extract_text_chars() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <text>Hello</text>
            <text><tspan>World</tspan></text>
        </svg>"#;
        let doc = parse_svg(svg).unwrap();
        let chars = extract_text_chars(&doc);
        assert!(chars.contains(&'H'));
        assert!(chars.contains(&'W'));
        assert!(!chars.contains(&'X'));
    }

    #[test]
    fn test_extract_font_faces() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <style>
                @font-face {
                    font-family: 'Iosevka';
                    src: url('fonts/Iosevka.woff2');
                    font-weight: bold;
                }
            </style>
        </svg>"#;
        let doc = parse_svg(svg).unwrap();
        let faces = extract_font_faces(&doc);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].family, "Iosevka");
        assert_eq!(faces[0].url, "fonts/Iosevka.woff2");
        assert_eq!(faces[0].weight, Some("bold".to_string()));
    }

    #[test]
    fn test_replace_font_url() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <style>@font-face { font-family: 'Test'; src: url('old.woff2'); }</style>
        </svg>"#;
        let mut doc = parse_svg(svg).unwrap();

        // Verify initial state
        let faces = extract_font_faces(&doc);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].url, "old.woff2");

        // Replace URL
        replace_font_url(&mut doc, "old.woff2", "new.woff2");

        // Verify replacement
        let faces = extract_font_faces(&doc);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].url, "new.woff2");
    }
}
