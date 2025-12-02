//! Image processing for responsive images
//!
//! Converts source images (PNG, JPG, GIF, WebP) to modern formats:
//! - JPEG-XL (best compression, future-proof)
//! - WebP (wide browser support, fallback)
//!
//! Also generates:
//! - Multiple width variants for srcset
//! - Thumbhash placeholders for instant loading

use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageEncoder, Rgb, Rgba};
use jpegxl_rs::encode::EncoderFrame;
use jxl_oxide::JxlImage;
use std::io::Cursor;

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
    let cursor = Cursor::new(data);

    match format {
        InputFormat::Png => image::codecs::png::PngDecoder::new(cursor)
            .ok()
            .map(|d| d.dimensions()),
        InputFormat::Jpg => image::codecs::jpeg::JpegDecoder::new(cursor)
            .ok()
            .map(|d| d.dimensions()),
        InputFormat::Gif => image::codecs::gif::GifDecoder::new(cursor)
            .ok()
            .map(|d| d.dimensions()),
        InputFormat::WebP => image::codecs::webp::WebPDecoder::new(cursor)
            .ok()
            .map(|d| d.dimensions()),
        InputFormat::Jxl => JxlImage::builder()
            .read(cursor)
            .ok()
            .map(|img| (img.width(), img.height())),
    }
}

/// Decode an image from bytes
fn decode_image(data: &[u8], format: InputFormat) -> Option<DynamicImage> {
    match format {
        InputFormat::Png => {
            image::load_from_memory_with_format(data, image::ImageFormat::Png).ok()
        }
        InputFormat::Jpg => {
            image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok()
        }
        InputFormat::Gif => {
            image::load_from_memory_with_format(data, image::ImageFormat::Gif).ok()
        }
        InputFormat::WebP => {
            image::load_from_memory_with_format(data, image::ImageFormat::WebP).ok()
        }
        InputFormat::Jxl => decode_jxl(data),
    }
}

/// Decode a JPEG-XL image using jxl-oxide
fn decode_jxl(data: &[u8]) -> Option<DynamicImage> {
    let image = JxlImage::builder().read(data).ok()?;
    let render = image.render_frame(0).ok()?;

    let mut stream = render.stream();
    let num_channels = stream.channels();
    let mut buffer = vec![0f32; (num_channels * stream.width() * stream.height()) as usize];
    stream.write_to_buffer(&mut buffer[..]);

    match num_channels {
        3 => {
            let img_buf = image::ImageBuffer::<Rgb<f32>, Vec<f32>>::from_raw(
                image.width(),
                image.height(),
                buffer,
            )?;
            Some(DynamicImage::from(img_buf))
        }
        4 => {
            let img_buf = image::ImageBuffer::<Rgba<f32>, Vec<f32>>::from_raw(
                image.width(),
                image.height(),
                buffer,
            )?;
            Some(DynamicImage::from(img_buf))
        }
        _ => None,
    }
}

/// Encode an image to WebP format
fn encode_webp(img: &DynamicImage) -> Option<Vec<u8>> {
    // WebP encoder only supports RGBA
    let rgba = img.to_rgba8();
    let rgba_img = DynamicImage::from(rgba);

    webp::Encoder::from_image(&rgba_img)
        .ok()?
        .encode(82.0) // Quality: 82 (good balance of quality/size)
        .to_vec()
        .into()
}

/// Encode an image to JPEG-XL format
fn encode_jxl(img: &DynamicImage) -> Option<Vec<u8>> {
    let runner = jpegxl_rs::ThreadsRunner::default();

    let mut encoder = jpegxl_rs::encoder_builder()
        .parallel_runner(&runner)
        .quality(2.8) // Distance metric (lower = better quality, 2.8 is high quality)
        .speed(jpegxl_rs::encode::EncoderSpeed::Squirrel) // Effort level 7
        .build()
        .ok()?;

    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        encoder.has_alpha = true;
        let frame = EncoderFrame::new(rgba.as_raw()).num_channels(4);
        encoder
            .encode_frame::<_, u8>(&frame, img.width(), img.height())
            .ok()
            .map(|r| r.data)
    } else {
        let rgb = img.to_rgb8();
        encoder.has_alpha = false;
        let frame = EncoderFrame::new(rgb.as_raw()).num_channels(3);
        encoder
            .encode_frame::<_, u8>(&frame, img.width(), img.height())
            .ok()
            .map(|r| r.data)
    }
}

/// Generate a thumbhash and encode it as a data URL
fn generate_thumbhash_data_url(img: &DynamicImage) -> Option<String> {
    // Thumbhash works best with small images, resize if needed
    let thumb_img = if img.width() > 100 || img.height() > 100 {
        img.resize(100, 100, image::imageops::FilterType::Triangle)
    } else {
        img.clone()
    };

    let rgba = thumb_img.to_rgba8();
    let hash = thumbhash::rgba_to_thumb_hash(
        thumb_img.width() as usize,
        thumb_img.height() as usize,
        rgba.as_raw(),
    );

    // Decode thumbhash back to RGBA for the placeholder image
    let (w, h, rgba_pixels) = thumbhash::thumb_hash_to_rgba(&hash).ok()?;

    // Create a tiny PNG from the decoded thumbhash
    let img_buf: image::RgbaImage = image::ImageBuffer::from_raw(w as u32, h as u32, rgba_pixels)?;

    let mut png_bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    encoder
        .write_image(
            img_buf.as_raw(),
            img_buf.width(),
            img_buf.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;

    // Encode as data URL
    let base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Some(format!("data:image/png;base64,{}", base64))
}

/// Resize an image maintaining aspect ratio
fn resize_image(img: &DynamicImage, target_width: u32) -> DynamicImage {
    let aspect = img.height() as f64 / img.width() as f64;
    let target_height = (target_width as f64 * aspect).round() as u32;
    img.resize_exact(
        target_width,
        target_height,
        image::imageops::FilterType::Lanczos3,
    )
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
    let img = decode_image(data, input_format)?;
    let (width, height) = (img.width(), img.height());
    let thumbhash_data_url = generate_thumbhash_data_url(&img)?;

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
    let img = decode_image(data, input_format)?;
    let (original_width, original_height) = (img.width(), img.height());

    // Generate thumbhash placeholder
    let thumbhash_data_url = generate_thumbhash_data_url(&img)?;

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
            img.clone()
        } else {
            resize_image(&img, width)
        };

        let height = resized.height();

        // Encode to both formats
        if let Some(jxl_data) = encode_jxl(&resized) {
            jxl_variants.push(ImageVariant {
                data: jxl_data,
                width,
                height,
            });
        }

        if let Some(webp_data) = encode_webp(&resized) {
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
