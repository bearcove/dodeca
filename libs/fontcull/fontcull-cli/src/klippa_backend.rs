use std::path::PathBuf;

use color_eyre::eyre::{Context, Result};

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

    // Decompress if WOFF/WOFF2
    let decompressed =
        fontcull::decompress_font(&font_data).map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    // Subset and compress to WOFF2
    let woff2_data = fontcull::subset_font_to_woff2_unicode(&decompressed, unicodes)
        .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;

    // Write the woff2 file
    std::fs::write(&output_path, &woff2_data)
        .wrap_err_with(|| format!("Failed to write subset font: {}", output_path.display()))?;

    Ok(output_path)
}
