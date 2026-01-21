//! Dodeca fonts cell (cell-fonts)
//!
//! This cell handles font subsetting and compression.
//! All CPU-intensive operations use spawn_blocking to enable parallelism.

use std::collections::HashSet;

use dodeca_cell_runtime::run_cell;
use tokio::task::spawn_blocking;

use cell_fonts_proto::{FontProcessor, FontProcessorDispatcher, FontResult, SubsetFontInput};

/// Font processor implementation
#[derive(Clone)]
pub struct FontProcessorImpl;

impl FontProcessor for FontProcessorImpl {
    async fn decompress_font(&self, data: Vec<u8>) -> FontResult {
        spawn_blocking(move || match fontcull::decompress_font(&data) {
            Ok(decompressed) => FontResult::DecompressSuccess { data: decompressed },
            Err(e) => FontResult::Error {
                message: format!("Failed to decompress font: {e}"),
            },
        })
        .await
        .unwrap_or_else(|e| FontResult::Error {
            message: format!("Task join error: {e}"),
        })
    }

    async fn subset_font(&self, input: SubsetFontInput) -> FontResult {
        spawn_blocking(move || {
            let char_set: HashSet<char> = input.chars.into_iter().collect();

            match fontcull::subset_font_data(&input.data, &char_set) {
                Ok(subsetted) => FontResult::SubsetSuccess { data: subsetted },
                Err(e) => FontResult::Error {
                    message: format!("Failed to subset font: {e}"),
                },
            }
        })
        .await
        .unwrap_or_else(|e| FontResult::Error {
            message: format!("Task join error: {e}"),
        })
    }

    async fn compress_to_woff2(&self, data: Vec<u8>) -> FontResult {
        spawn_blocking(move || match fontcull::compress_to_woff2(&data) {
            Ok(woff2) => FontResult::CompressSuccess { data: woff2 },
            Err(e) => FontResult::Error {
                message: format!("Failed to compress to WOFF2: {e}"),
            },
        })
        .await
        .unwrap_or_else(|e| FontResult::Error {
            message: format!("Task join error: {e}"),
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("fonts", |_handle| FontProcessorDispatcher::new(
        FontProcessorImpl
    ))
}
