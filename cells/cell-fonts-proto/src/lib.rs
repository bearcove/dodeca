//! RPC protocol for dodeca fonts cell
//!
//! Defines services for font analysis, subsetting, and compression.

use facet::Facet;
use std::collections::HashMap;

/// A parsed @font-face rule
#[derive(Debug, Clone, Facet)]
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
#[derive(Debug, Clone, Facet)]
pub struct FontAnalysis {
    /// Map of font-family name -> characters used (as sorted Vec for determinism)
    pub chars_per_font: HashMap<String, Vec<char>>,
    /// Parsed @font-face rules
    pub font_faces: Vec<FontFace>,
}

/// Input for font subsetting
#[derive(Debug, Clone, Facet)]
pub struct SubsetFontInput {
    pub data: Vec<u8>,
    pub chars: Vec<char>,
}

/// Result of font processing operations
#[derive(Debug, Clone, Facet)]
#[repr(u8)]
pub enum FontResult {
    /// Successfully analyzed fonts
    AnalysisSuccess { analysis: FontAnalysis },
    /// Successfully extracted CSS
    CssSuccess { css: String },
    /// Successfully decompressed font
    DecompressSuccess { data: Vec<u8> },
    /// Successfully subsetted font
    SubsetSuccess { data: Vec<u8> },
    /// Successfully compressed font
    CompressSuccess { data: Vec<u8> },
    /// Error during processing
    Error { message: String },
}

/// Font processing service implemented by the cell.
///
/// The host calls these methods to process fonts.
#[allow(async_fn_in_trait)]
#[rapace::service]
pub trait FontProcessor {
    /// Analyze HTML and CSS to collect font usage information
    async fn analyze_fonts(&self, html: String, css: String) -> FontResult;

    /// Extract inline CSS from HTML (from `<style>` tags)
    async fn extract_css_from_html(&self, html: String) -> FontResult;

    /// Decompress a WOFF2/WOFF font to TTF
    async fn decompress_font(&self, data: Vec<u8>) -> FontResult;

    /// Subset a font to only include specified characters
    async fn subset_font(&self, input: SubsetFontInput) -> FontResult;

    /// Compress TTF font data to WOFF2
    async fn compress_to_woff2(&self, data: Vec<u8>) -> FontResult;
}
