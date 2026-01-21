//! RPC protocol for dodeca fonts cell
//!
//! Defines services for font subsetting and compression.

use facet::Facet;

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
#[roam::service]
pub trait FontProcessor {
    /// Decompress a WOFF2/WOFF font to TTF
    async fn decompress_font(&self, data: Vec<u8>) -> FontResult;

    /// Subset a font to only include specified characters
    async fn subset_font(&self, input: SubsetFontInput) -> FontResult;

    /// Compress TTF font data to WOFF2
    async fn compress_to_woff2(&self, data: Vec<u8>) -> FontResult;
}
