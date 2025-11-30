use std::path::PathBuf;

use color_eyre::eyre::{Context, Result};
use fontcull_klippa::{Plan, SubsetFlags, subset_font};
use fontcull_read_fonts::collections::IntSet;
use fontcull_skrifa::{FontRef, GlyphId};
use fontcull_write_fonts::types::{NameId, Tag};
use woff::version2::compress;

/// Subset a font using klippa (pure Rust, no external dependencies)
pub fn subset_with_klippa(
    font_path: &str,
    unicodes: &[u32],
    output_dir: Option<&PathBuf>,
) -> Result<PathBuf> {
    let path = PathBuf::from(font_path);
    let stem = path.file_stem().unwrap().to_str().unwrap();

    let output_path = match output_dir {
        Some(dir) => dir.join(format!("{}-subset.woff2", stem)),
        None => path.with_file_name(format!("{}-subset.woff2", stem)),
    };

    // Read the input font
    let font_data = std::fs::read(font_path)
        .wrap_err_with(|| format!("Failed to read font file: {}", font_path))?;

    let font = FontRef::new(&font_data)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to parse font: {:?}", e))?;

    // Convert unicodes to IntSet
    let mut unicode_set = IntSet::<u32>::empty();
    for &u in unicodes {
        unicode_set.insert(u);
    }

    // Empty sets for optional parameters
    let glyph_ids = IntSet::<GlyphId>::empty();
    let drop_tables = IntSet::<Tag>::empty();
    let layout_scripts = IntSet::<Tag>::empty();
    let layout_features = IntSet::<Tag>::empty();
    let name_ids = IntSet::<NameId>::empty();
    let name_languages = IntSet::<u16>::empty();

    // Create subsetting plan - keep all layout features like pyftsubset does
    let plan = Plan::new(
        &glyph_ids,
        &unicode_set,
        &font,
        SubsetFlags::default(),
        &drop_tables,
        &layout_scripts,
        &layout_features,
        &name_ids,
        &name_languages,
    );

    // Perform subsetting
    let subset_data = subset_font(&font, &plan)
        .map_err(|e| color_eyre::eyre::eyre!("Failed to subset font: {:?}", e))?;

    // Compress to woff2 (metadata empty, quality 11 = max, transform = true for better compression)
    let woff2_data = compress(&subset_data, "", 11, true)
        .ok_or_else(|| color_eyre::eyre::eyre!("Failed to compress to woff2"))?;

    // Write the woff2 file
    std::fs::write(&output_path, &woff2_data)
        .wrap_err_with(|| format!("Failed to write subset font: {}", output_path.display()))?;

    Ok(output_path)
}
