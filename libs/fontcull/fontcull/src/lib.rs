#![doc = include_str!("../README.md")]

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
    /// Failed to decompress WOFF font
    WoffDecompress(String),
}

impl std::fmt::Display for SubsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubsetError::FontParse(msg) => write!(f, "failed to parse font: {msg}"),
            SubsetError::Subset(msg) => write!(f, "failed to subset font: {msg}"),
            SubsetError::Woff2(msg) => write!(f, "failed to compress to WOFF2: {msg}"),
            SubsetError::WoffDecompress(msg) => write!(f, "failed to decompress WOFF: {msg}"),
        }
    }
}

impl std::error::Error for SubsetError {}

/// The format of a font file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontFormat {
    /// TrueType font (.ttf)
    Ttf,
    /// OpenType font (.otf)
    Otf,
    /// WOFF (Web Open Font Format) version 1
    Woff,
    /// WOFF (Web Open Font Format) version 2
    Woff2,
    /// Unknown format
    Unknown,
}

impl FontFormat {
    /// Detect the format of a font file from its magic bytes
    pub fn detect(data: &[u8]) -> Self {
        if data.len() < 4 {
            return FontFormat::Unknown;
        }
        match &data[0..4] {
            // WOFF2: "wOF2" = 0x774F4632
            [0x77, 0x4F, 0x46, 0x32] => FontFormat::Woff2,
            // WOFF1: "wOFF" = 0x774F4646
            [0x77, 0x4F, 0x46, 0x46] => FontFormat::Woff,
            // TrueType: 0x00010000
            [0x00, 0x01, 0x00, 0x00] => FontFormat::Ttf,
            // OpenType with CFF: "OTTO"
            [0x4F, 0x54, 0x54, 0x4F] => FontFormat::Otf,
            // TrueType collection: "ttcf"
            [0x74, 0x74, 0x63, 0x66] => FontFormat::Ttf,
            // "true" (used by some Mac fonts)
            [0x74, 0x72, 0x75, 0x65] => FontFormat::Ttf,
            _ => FontFormat::Unknown,
        }
    }

    /// Returns true if this format is WOFF2
    pub fn is_woff2(&self) -> bool {
        matches!(self, FontFormat::Woff2)
    }
}

/// Decompress a WOFF2 font to TTF/OTF
///
/// If the input is already TTF/OTF, returns a copy unchanged.
/// If the input is WOFF2, decompresses it to TTF/OTF.
///
/// This is a separate operation that can be cached/salsified independently
/// from subsetting.
///
/// Requires the `woff2` feature (enabled by default).
#[cfg(feature = "woff2")]
pub fn decompress_font(font_data: &[u8]) -> Result<Vec<u8>, SubsetError> {
    match FontFormat::detect(font_data) {
        FontFormat::Woff2 => woofwoof::decompress(font_data)
            .ok_or_else(|| SubsetError::WoffDecompress("WOFF2 decompression failed".to_string())),
        FontFormat::Woff => Err(SubsetError::WoffDecompress(
            "WOFF1 decompression not supported, please convert to WOFF2 or TTF first".to_string(),
        )),
        // Already TTF/OTF, return as-is
        _ => Ok(font_data.to_vec()),
    }
}

/// Compress TTF/OTF font data to WOFF2
///
/// Uses maximum compression (level 11) with no embedded metadata.
///
/// This is a separate operation that can be cached/salsified independently
/// from subsetting.
///
/// Requires the `woff2` feature (enabled by default).
#[cfg(feature = "woff2")]
pub fn compress_to_woff2(font_data: &[u8]) -> Result<Vec<u8>, SubsetError> {
    // woofwoof::compress(data, metadata, quality, allow_transforms)
    // - metadata: empty string (no metadata)
    // - quality: 11 (maximum brotli compression)
    // - allow_transforms: true (enable WOFF2 table transforms)
    woofwoof::compress(font_data, "", 11, true)
        .ok_or_else(|| SubsetError::Woff2("WOFF2 compression failed".to_string()))
}

/// Subset a font to only include the specified characters
///
/// Takes raw font data (TTF/OTF/WOFF/WOFF2) and a set of characters,
/// returns the subsetted font as TTF bytes.
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

    // Tis done
    Ok(subsetted)
}

/// Subset a font and compress to WOFF2
///
/// Takes raw font data and a set of characters,
/// returns the subsetted font as WOFF2 bytes.
///
/// Requires the `woff2` feature (enabled by default).
#[cfg(feature = "woff2")]
pub fn subset_font_to_woff2(
    font_data: &[u8],
    chars: &HashSet<char>,
) -> Result<Vec<u8>, SubsetError> {
    let subsetted = subset_font_data(font_data, chars)?;

    // Compress to WOFF2
    let woff2 = woofwoof::compress(&subsetted, "", 11, true)
        .ok_or_else(|| SubsetError::Woff2("WOFF2 compression failed".to_string()))?;

    Ok(woff2)
}

/// Subset a font using unicode codepoints (u32) instead of chars
///
/// This is useful when you already have codepoints from browser extraction.
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
///
/// Requires the `woff2` feature (enabled by default).
#[cfg(feature = "woff2")]
pub fn subset_font_to_woff2_unicode(
    font_data: &[u8],
    unicodes: &[u32],
) -> Result<Vec<u8>, SubsetError> {
    let subsetted = subset_font_data_unicode(font_data, unicodes)?;

    let woff2 = woofwoof::compress(&subsetted, "", 11, true)
        .ok_or_else(|| SubsetError::Woff2("WOFF2 compression failed".to_string()))?;

    Ok(woff2)
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[test]
    fn test_subset_error_display() {
        let err = SubsetError::FontParse("invalid header".to_string());
        assert_eq!(format!("{}", err), "failed to parse font: invalid header");
    }

    #[test]
    fn test_font_format_detection() {
        // WOFF2 magic: "wOF2"
        assert_eq!(
            FontFormat::detect(&[0x77, 0x4F, 0x46, 0x32]),
            FontFormat::Woff2
        );
        // WOFF1 magic: "wOFF"
        assert_eq!(
            FontFormat::detect(&[0x77, 0x4F, 0x46, 0x46]),
            FontFormat::Woff
        );
        // TrueType magic: 0x00010000
        assert_eq!(
            FontFormat::detect(&[0x00, 0x01, 0x00, 0x00]),
            FontFormat::Ttf
        );
        // OpenType magic: "OTTO"
        assert_eq!(
            FontFormat::detect(&[0x4F, 0x54, 0x54, 0x4F]),
            FontFormat::Otf
        );
        // Too short
        assert_eq!(FontFormat::detect(&[0x00, 0x01]), FontFormat::Unknown);
        // Unknown
        assert_eq!(
            FontFormat::detect(&[0xDE, 0xAD, 0xBE, 0xEF]),
            FontFormat::Unknown
        );
    }

    #[test]
    fn test_font_format_is_woff2() {
        assert!(FontFormat::Woff2.is_woff2());
        assert!(!FontFormat::Woff.is_woff2());
        assert!(!FontFormat::Ttf.is_woff2());
        assert!(!FontFormat::Otf.is_woff2());
        assert!(!FontFormat::Unknown.is_woff2());
    }

    #[test]
    #[cfg(feature = "woff2")]
    fn test_decompress_ttf_passthrough() {
        // A minimal valid-ish TTF header (just for format detection)
        let ttf_data = [0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let result = decompress_font(&ttf_data).unwrap();
        assert_eq!(result, ttf_data);
    }

    #[test]
    #[cfg(feature = "woff2")]
    fn test_decompress_woff2_fixture() {
        // Read WOFF2 fixture file (created by fonttools)
        let woff2_data =
            std::fs::read("test_data/simple_glyf.woff2").expect("failed to read WOFF2 fixture");

        // Verify it's actually WOFF2
        assert_eq!(FontFormat::detect(&woff2_data), FontFormat::Woff2);

        // Decompress
        let decompressed = decompress_font(&woff2_data).expect("failed to decompress WOFF2");

        // The decompressed data should be valid TTF
        assert_eq!(FontFormat::detect(&decompressed), FontFormat::Ttf);

        // And we should be able to subset it
        let chars: HashSet<char> = ['a', 'b', 'c'].into_iter().collect();
        let _subsetted = subset_font_data(&decompressed, &chars).expect("failed to subset");
    }

    #[test]
    #[cfg(feature = "woff2")]
    fn test_decompress_woff1_not_supported() {
        // Read WOFF1 fixture file (created by fonttools)
        let woff1_data =
            std::fs::read("test_data/simple_glyf.woff").expect("failed to read WOFF1 fixture");

        // Verify it's actually WOFF1
        assert_eq!(FontFormat::detect(&woff1_data), FontFormat::Woff);

        // WOFF1 decompression is not supported with woofwoof
        let result = decompress_font(&woff1_data);
        assert!(result.is_err());
    }

    #[test]
    #[cfg(feature = "woff2")]
    fn test_subset_woff2_input() {
        // Read WOFF2 fixture
        let woff2_input =
            std::fs::read("test_data/simple_glyf.woff2").expect("failed to read WOFF2 fixture");

        // Decompress, subset, and recompress - the full pipeline
        let decompressed = decompress_font(&woff2_input).expect("failed to decompress");
        let chars: HashSet<char> = ['a', 'b', 'c'].into_iter().collect();
        let subsetted = subset_font_data(&decompressed, &chars).expect("failed to subset");
        let woff2_output = compress_to_woff2(&subsetted).expect("failed to compress output");

        // Verify output is WOFF2
        assert_eq!(FontFormat::detect(&woff2_output), FontFormat::Woff2);
    }
}
