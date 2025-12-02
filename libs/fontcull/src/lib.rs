//! fontcull - Font subsetting library
//!
//! Subset fonts to only include glyphs that are actually used.
//!
//! # Features
//!
//! - `klippa` (default): Pure Rust font subsetting with klippa
//! - `static-analysis`: Static HTML/CSS parsing for font usage detection
//! - `browser`: Browser-based glyph extraction with chromiumoxide
//!
//! # Example (static analysis)
//!
//! ```ignore
//! use fontcull::{analyze_fonts, subset_font_to_woff2, extract_css_from_html};
//!
//! let html = r#"<html><head><style>body { font-family: "MyFont"; }</style></head>
//!               <body><p>Hello World</p></body></html>"#;
//!
//! let css = extract_css_from_html(html);
//! let analysis = analyze_fonts(html, &css);
//!
//! // Get chars used by "MyFont"
//! if let Some(chars) = analysis.chars_per_font.get("MyFont") {
//!     let font_data = std::fs::read("MyFont.ttf").unwrap();
//!     let woff2 = subset_font_to_woff2(&font_data, chars).unwrap();
//!     std::fs::write("MyFont-subset.woff2", woff2).unwrap();
//! }
//! ```

use std::collections::HashSet;

#[cfg(feature = "static-analysis")]
mod static_analysis;

#[cfg(feature = "static-analysis")]
pub use static_analysis::*;

/// Error type for font subsetting
#[derive(Debug)]
pub enum SubsetError {
    /// Failed to parse font file
    FontParse(String),
    /// Failed to subset font
    Subset(String),
    /// Failed to compress to WOFF2
    Woff2(String),
}

impl std::fmt::Display for SubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubsetError::FontParse(msg) => write!(f, "failed to parse font: {msg}"),
            SubsetError::Subset(msg) => write!(f, "failed to subset font: {msg}"),
            SubsetError::Woff2(msg) => write!(f, "failed to compress to WOFF2: {msg}"),
        }
    }
}

impl std::error::Error for SubsetError {}

/// Subset a font to only include the specified characters
///
/// Takes raw font data (TTF/OTF/WOFF/WOFF2) and a set of characters,
/// returns the subsetted font as TTF bytes.
#[cfg(feature = "klippa")]
pub fn subset_font_data(font_data: &[u8], chars: &HashSet<char>) -> Result<Vec<u8>, SubsetError> {
    use fontcull_klippa::{Plan, SubsetFlags, subset_font};
    use fontcull_read_fonts::collections::IntSet;
    use fontcull_skrifa::{FontRef, GlyphId, Tag};
    use fontcull_write_fonts::types::NameId;

    // Parse the font
    let font = FontRef::new(font_data).map_err(|e| SubsetError::FontParse(format!("{e:?}")))?;

    // Convert chars to unicode codepoints
    let mut unicodes: IntSet<u32> = IntSet::empty();
    for c in chars {
        unicodes.insert(*c as u32);
    }

    // Empty sets for optional parameters
    let empty_gids: IntSet<GlyphId> = IntSet::empty();
    let empty_tags: IntSet<Tag> = IntSet::empty();
    let empty_name_ids: IntSet<NameId> = IntSet::empty();
    let empty_langs: IntSet<u16> = IntSet::empty();

    // Create subsetting plan
    let plan = Plan::new(
        &empty_gids, // glyph IDs - not needed when using unicodes
        &unicodes,   // unicode codepoints to keep
        &font,
        SubsetFlags::default(),
        &empty_tags,     // tables to drop
        &empty_tags,     // layout scripts
        &empty_tags,     // layout features
        &empty_name_ids, // name IDs
        &empty_langs,    // name languages
    );

    // Perform subsetting
    let subsetted = subset_font(&font, &plan).map_err(|e| SubsetError::Subset(format!("{e:?}")))?;

    Ok(subsetted)
}

/// Subset a font and compress to WOFF2
///
/// Takes raw font data and a set of characters,
/// returns the subsetted font as WOFF2 bytes.
#[cfg(feature = "klippa")]
pub fn subset_font_to_woff2(
    font_data: &[u8],
    chars: &HashSet<char>,
) -> Result<Vec<u8>, SubsetError> {
    let subsetted = subset_font_data(font_data, chars)?;

    // Compress to WOFF2
    let woff2 = woff::version2::compress(&subsetted, "", 11, true)
        .ok_or_else(|| SubsetError::Woff2("WOFF2 compression failed".to_string()))?;

    Ok(woff2)
}

/// Subset a font using unicode codepoints (u32) instead of chars
///
/// This is useful when you already have codepoints from browser extraction.
#[cfg(feature = "klippa")]
pub fn subset_font_data_unicode(
    font_data: &[u8],
    unicodes: &[u32],
) -> Result<Vec<u8>, SubsetError> {
    use fontcull_klippa::{Plan, SubsetFlags, subset_font};
    use fontcull_read_fonts::collections::IntSet;
    use fontcull_skrifa::{FontRef, GlyphId, Tag};
    use fontcull_write_fonts::types::NameId;

    let font = FontRef::new(font_data).map_err(|e| SubsetError::FontParse(format!("{e:?}")))?;

    let mut unicode_set: IntSet<u32> = IntSet::empty();
    for &u in unicodes {
        unicode_set.insert(u);
    }

    let empty_gids: IntSet<GlyphId> = IntSet::empty();
    let empty_tags: IntSet<Tag> = IntSet::empty();
    let empty_name_ids: IntSet<NameId> = IntSet::empty();
    let empty_langs: IntSet<u16> = IntSet::empty();

    let plan = Plan::new(
        &empty_gids,
        &unicode_set,
        &font,
        SubsetFlags::default(),
        &empty_tags,
        &empty_tags,
        &empty_tags,
        &empty_name_ids,
        &empty_langs,
    );

    let subsetted = subset_font(&font, &plan).map_err(|e| SubsetError::Subset(format!("{e:?}")))?;

    Ok(subsetted)
}

/// Subset a font to WOFF2 using unicode codepoints (u32)
#[cfg(feature = "klippa")]
pub fn subset_font_to_woff2_unicode(
    font_data: &[u8],
    unicodes: &[u32],
) -> Result<Vec<u8>, SubsetError> {
    let subsetted = subset_font_data_unicode(font_data, unicodes)?;

    let woff2 = woff::version2::compress(&subsetted, "", 11, true)
        .ok_or_else(|| SubsetError::Woff2("WOFF2 compression failed".to_string()))?;

    Ok(woff2)
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    #[cfg(feature = "klippa")]
    fn test_subset_error_display() {
        let err = SubsetError::FontParse("invalid header".to_string());
        assert_eq!(format!("{}", err), "failed to parse font: invalid header");
    }
}
