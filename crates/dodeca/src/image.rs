//! Image processing for responsive images
//!
//! Converts source images (PNG, JPG, GIF, WebP, JXL) to modern formats:
//! - JPEG-XL (best compression, future-proof)
//! - WebP (wide browser support, fallback)
//!
//! All image processing (decoding, resizing, thumbhash) is done via plugins.
//!
//! Also generates:
//! - Multiple width variants for srcset
//! - Thumbhash placeholders for instant loading

use crate::plugins::{self, DecodedImage};

/// Standard responsive breakpoints (in pixels)
/// Only widths smaller than the original will be generated
pub const RESPONSIVE_WIDTHS: &[u32] = &[320, 640, 960, 1280, 1920];

/// Supported input image formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputFormat {
    Png,
    Jpg,
    Gif,
    WebP,
    Jxl,
}

impl InputFormat {
    /// Detect format from file extension
    pub fn from_extension(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".png") {
            Some(Self::Png)
        } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
            Some(Self::Jpg)
        } else if lower.ends_with(".gif") {
            Some(Self::Gif)
        } else if lower.ends_with(".webp") {
            Some(Self::WebP)
        } else if lower.ends_with(".jxl") {
            Some(Self::Jxl)
        } else {
            None
        }
    }

    /// Check if this is a processable image format
    pub fn is_processable(path: &str) -> bool {
        Self::from_extension(path).is_some()
    }
}

/// Output image format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// JPEG-XL - best compression, modern browsers
    Jxl,
    /// WebP - wide browser support, fallback
    WebP,
}

impl OutputFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jxl => "jxl",
            Self::WebP => "webp",
        }
    }
}

/// A single image variant (one format, one size)
#[derive(Debug, Clone)]
pub struct ImageVariant {
    /// The encoded image data
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

/// Complete result of processing an image
#[derive(Debug, Clone)]
pub struct ProcessedImageSet {
    /// Original width
    pub original_width: u32,
    /// Original height
    pub original_height: u32,
    /// Thumbhash as base64 data URL (tiny PNG placeholder)
    pub thumbhash_data_url: String,
    /// JXL variants at different widths (sorted by width ascending)
    pub jxl_variants: Vec<ImageVariant>,
    /// WebP variants at different widths (sorted by width ascending)
    pub webp_variants: Vec<ImageVariant>,
}

/// Get dimensions of an image without fully decoding it
#[allow(dead_code)]
pub fn get_dimensions(data: &[u8], format: InputFormat) -> Option<(u32, u32)> {
    let decoded = decode_image(data, format)?;
    Some((decoded.width, decoded.height))
}

/// Decode an image from bytes using the appropriate plugin
fn decode_image(data: &[u8], format: InputFormat) -> Option<DecodedImage> {
    match format {
        InputFormat::Png => plugins::decode_png_plugin(data),
        InputFormat::Jpg => plugins::decode_jpeg_plugin(data),
        InputFormat::Gif => plugins::decode_gif_plugin(data),
        InputFormat::WebP => plugins::decode_webp_plugin(data),
        InputFormat::Jxl => plugins::decode_jxl_plugin(data),
    }
}

/// Resize an image to a target width, maintaining aspect ratio
fn resize_image(decoded: &DecodedImage, target_width: u32) -> Option<DecodedImage> {
    plugins::resize_image_plugin(
        &decoded.pixels,
        decoded.width,
        decoded.height,
        decoded.channels,
        target_width,
    )
}

/// Generate a thumbhash and encode it as a data URL
fn generate_thumbhash_data_url(decoded: &DecodedImage) -> Option<String> {
    plugins::generate_thumbhash_plugin(&decoded.pixels, decoded.width, decoded.height)
}

/// Encode pixels to WebP format (via plugin)
fn encode_webp(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    plugins::encode_webp_plugin(pixels, width, height, 82)
}

/// Encode pixels to JPEG-XL format (via plugin)
fn encode_jxl(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    // Quality 80 maps to distance ~3 in the plugin (high quality)
    plugins::encode_jxl_plugin(pixels, width, height, 80)
}

/// Image metadata without the processed bytes
/// This is fast to compute (decode only, no encode)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageMetadata {
    /// Original width
    pub width: u32,
    /// Original height
    pub height: u32,
    /// Thumbhash as base64 data URL
    pub thumbhash_data_url: String,
    /// Which widths we'll generate variants for
    pub variant_widths: Vec<u32>,
}

/// Get image metadata without processing (fast - decode only, no encode)
pub fn get_image_metadata(data: &[u8], input_format: InputFormat) -> Option<ImageMetadata> {
    let decoded = decode_image(data, input_format)?;
    let (width, height) = (decoded.width, decoded.height);
    let thumbhash_data_url = generate_thumbhash_data_url(&decoded)?;

    // Compute which widths we'll generate (same logic as process_image)
    let mut variant_widths: Vec<u32> = RESPONSIVE_WIDTHS
        .iter()
        .copied()
        .filter(|&w| w < width)
        .collect();
    variant_widths.push(width); // Always include original
    variant_widths.sort();

    Some(ImageMetadata {
        width,
        height,
        thumbhash_data_url,
        variant_widths,
    })
}

/// Process an image and generate all variants
///
/// Returns None if the image cannot be processed (unsupported format, decode error, etc.)
pub fn process_image(data: &[u8], input_format: InputFormat) -> Option<ProcessedImageSet> {
    let decoded = decode_image(data, input_format)?;
    let (original_width, original_height) = (decoded.width, decoded.height);

    // Generate thumbhash placeholder
    let thumbhash_data_url = generate_thumbhash_data_url(&decoded)?;

    // Determine which widths to generate (only those smaller than original, plus original)
    let mut widths: Vec<u32> = RESPONSIVE_WIDTHS
        .iter()
        .copied()
        .filter(|&w| w < original_width)
        .collect();
    widths.push(original_width); // Always include original size
    widths.sort();
    widths.dedup();

    // Generate variants for each width
    let mut jxl_variants = Vec::new();
    let mut webp_variants = Vec::new();

    for &width in &widths {
        let resized = if width == original_width {
            // Use original decoded image
            DecodedImage {
                pixels: decoded.pixels.clone(),
                width: decoded.width,
                height: decoded.height,
                channels: decoded.channels,
            }
        } else {
            resize_image(&decoded, width)?
        };

        let height = resized.height;

        // Encode to both formats
        if let Some(jxl_data) = encode_jxl(&resized.pixels, resized.width, resized.height) {
            jxl_variants.push(ImageVariant {
                data: jxl_data,
                width,
                height,
            });
        }

        if let Some(webp_data) = encode_webp(&resized.pixels, resized.width, resized.height) {
            webp_variants.push(ImageVariant {
                data: webp_data,
                width,
                height,
            });
        }
    }

    // Ensure we have at least one variant of each format
    if jxl_variants.is_empty() || webp_variants.is_empty() {
        return None;
    }

    Some(ProcessedImageSet {
        original_width,
        original_height,
        thumbhash_data_url,
        jxl_variants,
        webp_variants,
    })
}

/// Change a file path's extension to a new format
pub fn change_extension(path: &str, new_ext: &str) -> String {
    if let Some(dot_pos) = path.rfind('.') {
        format!("{}.{}", &path[..dot_pos], new_ext)
    } else {
        format!("{}.{}", path, new_ext)
    }
}

/// Add width suffix to a path (before extension)
/// e.g., "photo.png" with width 640 -> "photo-640w.png"
pub fn add_width_suffix(path: &str, width: u32) -> String {
    if let Some(dot_pos) = path.rfind('.') {
        format!("{}-{}w{}", &path[..dot_pos], width, &path[dot_pos..])
    } else {
        format!("{}-{}w", path, width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(
            InputFormat::from_extension("image.png"),
            Some(InputFormat::Png)
        );
        assert_eq!(
            InputFormat::from_extension("image.PNG"),
            Some(InputFormat::Png)
        );
        assert_eq!(
            InputFormat::from_extension("image.jpg"),
            Some(InputFormat::Jpg)
        );
        assert_eq!(
            InputFormat::from_extension("image.jpeg"),
            Some(InputFormat::Jpg)
        );
        assert_eq!(
            InputFormat::from_extension("image.gif"),
            Some(InputFormat::Gif)
        );
        assert_eq!(
            InputFormat::from_extension("image.webp"),
            Some(InputFormat::WebP)
        );
        assert_eq!(
            InputFormat::from_extension("image.jxl"),
            Some(InputFormat::Jxl)
        );
        assert_eq!(InputFormat::from_extension("image.svg"), None);
        assert_eq!(InputFormat::from_extension("image.txt"), None);
    }

    #[test]
    fn test_change_extension() {
        assert_eq!(
            change_extension("images/photo.png", "webp"),
            "images/photo.webp"
        );
        assert_eq!(
            change_extension("images/photo.jpg", "jxl"),
            "images/photo.jxl"
        );
        assert_eq!(change_extension("no_ext", "webp"), "no_ext.webp");
    }

    #[test]
    fn test_add_width_suffix() {
        assert_eq!(
            add_width_suffix("images/photo.png", 640),
            "images/photo-640w.png"
        );
        assert_eq!(
            add_width_suffix("photo.jpg", 1280),
            "photo-1280w.jpg"
        );
        assert_eq!(add_width_suffix("no_ext", 320), "no_ext-320w");
    }

    #[test]
    fn test_output_format() {
        assert_eq!(OutputFormat::Jxl.extension(), "jxl");
        assert_eq!(OutputFormat::WebP.extension(), "webp");
    }
}
