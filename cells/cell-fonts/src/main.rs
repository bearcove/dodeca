//! Dodeca fonts cell (cell-fonts)
//!
//! This cell handles font analysis, subsetting, and compression.

use std::collections::{HashMap, HashSet};

use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;

use cell_fonts_proto::{
    FontAnalysis, FontFace, FontProcessor, FontProcessorDispatcher, FontResult, SubsetFontInput,
};

/// Font processor implementation
#[derive(Clone)]
pub struct FontProcessorImpl;

impl FontProcessor for FontProcessorImpl {
    async fn analyze_fonts(&self, html: String, css: String) -> FontResult {
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

        FontResult::AnalysisSuccess {
            analysis: FontAnalysis {
                chars_per_font,
                font_faces,
            },
        }
    }

    async fn extract_css_from_html(&self, html: String) -> FontResult {
        FontResult::CssSuccess {
            css: fontcull::extract_css_from_html(&html),
        }
    }

    async fn decompress_font(&self, data: Vec<u8>) -> FontResult {
        match fontcull::decompress_font(&data) {
            Ok(decompressed) => FontResult::DecompressSuccess { data: decompressed },
            Err(e) => FontResult::Error {
                message: format!("Failed to decompress font: {e}"),
            },
        }
    }

    async fn subset_font(&self, input: SubsetFontInput) -> FontResult {
        let char_set: HashSet<char> = input.chars.into_iter().collect();

        match fontcull::subset_font_data(&input.data, &char_set) {
            Ok(subsetted) => FontResult::SubsetSuccess { data: subsetted },
            Err(e) => FontResult::Error {
                message: format!("Failed to subset font: {e}"),
            },
        }
    }

    async fn compress_to_woff2(&self, data: Vec<u8>) -> FontResult {
        match fontcull::compress_to_woff2(&data) {
            Ok(woff2) => FontResult::CompressSuccess { data: woff2 },
            Err(e) => FontResult::Error {
                message: format!("Failed to compress to WOFF2: {e}"),
            },
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = FontProcessorDispatcher::new(FontProcessorImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await;
    Ok(())
}
