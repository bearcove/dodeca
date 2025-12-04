//! Font processing plugin for dodeca
//!
//! Handles font analysis, subsetting, and compression.
//! This moves the `fontcull` crate out of the main binary.

use facet::Facet;
use plugcard::{plugcard, PlugResult};
use std::collections::{HashMap, HashSet};

plugcard::export_plugin!();

/// A parsed @font-face rule
#[derive(Facet, Debug, Clone)]
pub struct FontFace {
    /// The font-family name declared in @font-face
    pub family: String,
    /// The URL to the font file (from src)
    pub src: String,
    /// Font weight (e.g., "400", "bold")
    pub weight: Option<String>,
    /// Font style (e.g., "normal", "italic")
    pub style: Option<String>,
}

/// Result of analyzing CSS for font information
#[derive(Facet, Debug, Clone)]
pub struct FontAnalysis {
    /// Map of font-family name -> characters used (as sorted Vec for determinism)
    pub chars_per_font: HashMap<String, Vec<char>>,
    /// Parsed @font-face rules
    pub font_faces: Vec<FontFace>,
}

/// Input for font analysis
#[derive(Facet)]
struct AnalyzeFontsInput {
    html: String,
    css: String,
}

/// Analyze HTML and CSS to collect font usage information
#[plugcard]
pub fn analyze_fonts(html: String, css: String) -> PlugResult<FontAnalysis> {
    let analysis = fontcull::analyze_fonts(&html, &css);

    // Convert HashSet<char> to sorted Vec<char> for deterministic serialization
    let chars_per_font: HashMap<String, Vec<char>> = analysis
        .chars_per_font
        .into_iter()
        .map(|(family, chars)| {
            let mut chars_vec: Vec<char> = chars.into_iter().collect();
            chars_vec.sort();
            (family, chars_vec)
        })
        .collect();

    let font_faces: Vec<FontFace> = analysis
        .font_faces
        .into_iter()
        .map(|f| FontFace {
            family: f.family,
            src: f.src,
            weight: f.weight,
            style: f.style,
        })
        .collect();

    PlugResult::Ok(FontAnalysis {
        chars_per_font,
        font_faces,
    })
}

/// Extract inline CSS from HTML (from <style> tags)
#[plugcard]
pub fn extract_css_from_html(html: String) -> PlugResult<String> {
    PlugResult::Ok(fontcull::extract_css_from_html(&html))
}

/// Decompress a WOFF2/WOFF font to TTF
#[plugcard]
pub fn decompress_font(data: Vec<u8>) -> PlugResult<Vec<u8>> {
    match fontcull::decompress_font(&data) {
        Ok(decompressed) => PlugResult::Ok(decompressed),
        Err(e) => PlugResult::Err(format!("Failed to decompress font: {e}")),
    }
}

/// Input for font subsetting
#[derive(Facet)]
struct SubsetFontInput {
    data: Vec<u8>,
    chars: Vec<char>,
}

/// Subset a font to only include specified characters
/// Input should be decompressed TTF data
#[plugcard]
pub fn subset_font(data: Vec<u8>, chars: Vec<char>) -> PlugResult<Vec<u8>> {
    let char_set: HashSet<char> = chars.into_iter().collect();

    match fontcull::subset_font_data(&data, &char_set) {
        Ok(subsetted) => PlugResult::Ok(subsetted),
        Err(e) => PlugResult::Err(format!("Failed to subset font: {e}")),
    }
}

/// Compress TTF font data to WOFF2
#[plugcard]
pub fn compress_to_woff2(data: Vec<u8>) -> PlugResult<Vec<u8>> {
    match fontcull::compress_to_woff2(&data) {
        Ok(woff2) => PlugResult::Ok(woff2),
        Err(e) => PlugResult::Err(format!("Failed to compress to WOFF2: {e}")),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_fonts() {
        let html = r#"<html><body><p style="font-family: Inter">Hello</p></body></html>"#;
        let css = r#"
            @font-face {
                font-family: "Inter";
                src: url("/fonts/Inter.woff2");
            }
        "#;

        let result = analyze_fonts(html.to_string(), css.to_string());
        let PlugResult::Ok(analysis) = result else {
            panic!("Expected Ok");
        };

        assert!(!analysis.font_faces.is_empty());
        assert_eq!(analysis.font_faces[0].family, "Inter");
    }

    #[test]
    fn test_extract_css_from_html() {
        let html = r#"<html><head><style>body { color: red; }</style></head></html>"#;

        let result = extract_css_from_html(html.to_string());
        let PlugResult::Ok(css) = result else {
            panic!("Expected Ok");
        };

        assert!(css.contains("color: red"));
    }
}
