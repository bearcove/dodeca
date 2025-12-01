//! Image processing for responsive images
//!
//! Converts source images (PNG, JPG, GIF, WebP) to modern formats:
//! - JPEG-XL (best compression, future-proof)
//! - WebP (wide browser support, fallback)

use image::{DynamicImage, ImageDecoder, Rgb, Rgba};
use jpegxl_rs::encode::EncoderFrame;
use jxl_oxide::JxlImage;
use std::io::Cursor;

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

/// Result of processing an image
#[derive(Debug, Clone)]
pub struct ProcessedImage {
    /// The encoded image data
    pub data: Vec<u8>,
    /// Original width in pixels
    pub width: u32,
    /// Original height in pixels
    pub height: u32,
}

/// Get dimensions of an image without fully decoding it
#[allow(dead_code)]
pub fn get_dimensions(data: &[u8], format: InputFormat) -> Option<(u32, u32)> {
    let cursor = Cursor::new(data);

    match format {
        InputFormat::Png => {
            image::codecs::png::PngDecoder::new(cursor)
                .ok()
                .map(|d| d.dimensions())
        }
        InputFormat::Jpg => {
            image::codecs::jpeg::JpegDecoder::new(cursor)
                .ok()
                .map(|d| d.dimensions())
        }
        InputFormat::Gif => {
            image::codecs::gif::GifDecoder::new(cursor)
                .ok()
                .map(|d| d.dimensions())
        }
        InputFormat::WebP => {
            image::codecs::webp::WebPDecoder::new(cursor)
                .ok()
                .map(|d| d.dimensions())
        }
        InputFormat::Jxl => {
            JxlImage::builder()
                .read(cursor)
                .ok()
                .map(|img| (img.width(), img.height()))
        }
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

/// Process an image and generate both JXL and WebP variants
///
/// Returns None if the image cannot be processed (unsupported format, decode error, etc.)
pub fn process_image(data: &[u8], input_format: InputFormat) -> Option<(ProcessedImage, ProcessedImage)> {
    let img = decode_image(data, input_format)?;
    let (width, height) = (img.width(), img.height());

    let jxl_data = encode_jxl(&img)?;
    let webp_data = encode_webp(&img)?;

    let jxl = ProcessedImage {
        data: jxl_data,
        width,
        height,
    };

    let webp = ProcessedImage {
        data: webp_data,
        width,
        height,
    };

    Some((jxl, webp))
}

/// Change a file path's extension to a new format
pub fn change_extension(path: &str, new_ext: &str) -> String {
    if let Some(dot_pos) = path.rfind('.') {
        format!("{}.{}", &path[..dot_pos], new_ext)
    } else {
        format!("{}.{}", path, new_ext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(InputFormat::from_extension("image.png"), Some(InputFormat::Png));
        assert_eq!(InputFormat::from_extension("image.PNG"), Some(InputFormat::Png));
        assert_eq!(InputFormat::from_extension("image.jpg"), Some(InputFormat::Jpg));
        assert_eq!(InputFormat::from_extension("image.jpeg"), Some(InputFormat::Jpg));
        assert_eq!(InputFormat::from_extension("image.gif"), Some(InputFormat::Gif));
        assert_eq!(InputFormat::from_extension("image.webp"), Some(InputFormat::WebP));
        assert_eq!(InputFormat::from_extension("image.jxl"), Some(InputFormat::Jxl));
        assert_eq!(InputFormat::from_extension("image.svg"), None);
        assert_eq!(InputFormat::from_extension("image.txt"), None);
    }

    #[test]
    fn test_change_extension() {
        assert_eq!(change_extension("images/photo.png", "webp"), "images/photo.webp");
        assert_eq!(change_extension("images/photo.jpg", "jxl"), "images/photo.jxl");
        assert_eq!(change_extension("no_ext", "webp"), "no_ext.webp");
    }

    #[test]
    fn test_output_format() {
        assert_eq!(OutputFormat::Jxl.extension(), "jxl");
        assert_eq!(OutputFormat::WebP.extension(), "webp");
    }
}
