//! Image processing plugin for dodeca
//!
//! Handles image decoding (PNG, JPG, GIF), resizing, and thumbhash generation.
//! This moves the `image` and `thumbhash` crates out of the main binary.

use base64::Engine;
use facet::Facet;
use image::{DynamicImage, ImageEncoder, Rgb, Rgba};
use plugcard::{plugcard, PlugResult};

plugcard::export_plugin!();

/// Decoded image data
#[derive(Facet)]
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
}

/// Decode a PNG image to RGBA pixels
#[plugcard]
pub fn decode_png(data: Vec<u8>) -> PlugResult<DecodedImage> {
    decode_format(&data, image::ImageFormat::Png)
}

/// Decode a JPEG image to RGBA pixels
#[plugcard]
pub fn decode_jpeg(data: Vec<u8>) -> PlugResult<DecodedImage> {
    decode_format(&data, image::ImageFormat::Jpeg)
}

/// Decode a GIF image to RGBA pixels (first frame only)
#[plugcard]
pub fn decode_gif(data: Vec<u8>) -> PlugResult<DecodedImage> {
    decode_format(&data, image::ImageFormat::Gif)
}

fn decode_format(data: &[u8], format: image::ImageFormat) -> PlugResult<DecodedImage> {
    let img = match image::load_from_memory_with_format(data, format) {
        Ok(img) => img,
        Err(e) => return PlugResult::Err(format!("Failed to decode image: {e}")),
    };

    let rgba = img.to_rgba8();
    PlugResult::Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
        channels: 4,
    })
}

/// Resize an image maintaining aspect ratio using Lanczos3 filter
#[plugcard]
pub fn resize_image(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    channels: u8,
    target_width: u32,
) -> PlugResult<DecodedImage> {
    let img = match pixels_to_dynamic_image(&pixels, width, height, channels) {
        Some(img) => img,
        None => return PlugResult::Err("Invalid pixel data".to_string()),
    };

    // Maintain aspect ratio
    let aspect = height as f64 / width as f64;
    let target_height = (target_width as f64 * aspect).round() as u32;

    let resized = img.resize_exact(
        target_width,
        target_height,
        image::imageops::FilterType::Lanczos3,
    );

    let rgba = resized.to_rgba8();
    PlugResult::Ok(DecodedImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
        channels: 4,
    })
}

/// Generate a thumbhash data URL from RGBA pixels
///
/// The thumbhash is a compact placeholder (~28 bytes) that can be decoded
/// to a blurry preview image. Returns a PNG data URL.
#[plugcard]
pub fn generate_thumbhash_data_url(pixels: Vec<u8>, width: u32, height: u32) -> PlugResult<String> {
    let img = match pixels_to_dynamic_image(&pixels, width, height, 4) {
        Some(img) => img,
        None => return PlugResult::Err("Invalid pixel data".to_string()),
    };

    // Thumbhash works best with small images, resize if needed
    let thumb_img = if width > 100 || height > 100 {
        img.resize(100, 100, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let rgba = thumb_img.to_rgba8();
    let hash = thumbhash::rgba_to_thumb_hash(
        thumb_img.width() as usize,
        thumb_img.height() as usize,
        rgba.as_raw(),
    );

    // Decode thumbhash back to RGBA for the placeholder image
    let (w, h, rgba_pixels) = match thumbhash::thumb_hash_to_rgba(&hash) {
        Ok(result) => result,
        Err(()) => return PlugResult::Err("Failed to decode thumbhash".to_string()),
    };

    // Create a tiny PNG from the decoded thumbhash
    let img_buf: image::RgbaImage = match image::ImageBuffer::from_raw(w as u32, h as u32, rgba_pixels) {
        Some(buf) => buf,
        None => return PlugResult::Err("Failed to create image buffer".to_string()),
    };

    let mut png_bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    if let Err(e) = encoder.write_image(
        img_buf.as_raw(),
        img_buf.width(),
        img_buf.height(),
        image::ExtendedColorType::Rgba8,
    ) {
        return PlugResult::Err(format!("Failed to encode PNG: {e}"));
    }

    // Encode as data URL
    let base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    PlugResult::Ok(format!("data:image/png;base64,{}", base64))
}

/// Convert raw pixels to DynamicImage
fn pixels_to_dynamic_image(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u8,
) -> Option<DynamicImage> {
    match channels {
        3 => {
            let img_buf =
                image::ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())?;
            Some(DynamicImage::from(img_buf))
        }
        4 => {
            let img_buf =
                image::ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())?;
            Some(DynamicImage::from(img_buf))
        }
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_image() {
        // 4x4 red pixels (RGBA)
        let pixels = vec![255u8, 0, 0, 255].repeat(4 * 4);

        let result = resize_image(pixels, 4, 4, 4, 2);
        let PlugResult::Ok(resized) = result else {
            panic!("Expected Ok, got Err");
        };

        assert_eq!(resized.width, 2);
        assert_eq!(resized.height, 2);
        assert_eq!(resized.channels, 4);
        assert_eq!(resized.pixels.len(), 2 * 2 * 4);
    }

    #[test]
    fn test_generate_thumbhash() {
        // 8x8 gradient pixels (RGBA)
        let mut pixels = Vec::with_capacity(8 * 8 * 4);
        for y in 0..8 {
            for x in 0..8 {
                pixels.push((x * 32) as u8); // R
                pixels.push((y * 32) as u8); // G
                pixels.push(128); // B
                pixels.push(255); // A
            }
        }

        let result = generate_thumbhash_data_url(pixels, 8, 8);
        let PlugResult::Ok(data_url) = result else {
            panic!("Expected Ok, got Err");
        };

        assert!(data_url.starts_with("data:image/png;base64,"));
    }
}
