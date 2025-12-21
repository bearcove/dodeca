//! Dodeca image cell (cell-image)
//!
//! This cell handles image decoding, resizing, and thumbhash generation.

use base64::Engine;
use image::{DynamicImage, ImageEncoder, Rgb, Rgba};

use cell_image_proto::{
    DecodedImage, ImageProcessor, ImageProcessorServer, ImageResult, ResizeInput, ThumbhashInput,
};

/// Image processor implementation
pub struct ImageProcessorImpl;

impl ImageProcessor for ImageProcessorImpl {
    async fn decode_png(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Png)
    }

    async fn decode_jpeg(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Jpeg)
    }

    async fn decode_gif(&self, data: Vec<u8>) -> ImageResult {
        decode_format(&data, image::ImageFormat::Gif)
    }

    async fn resize_image(&self, input: ResizeInput) -> ImageResult {
        let img =
            match pixels_to_dynamic_image(&input.pixels, input.width, input.height, input.channels)
            {
                Some(img) => img,
                None => {
                    return ImageResult::Error {
                        message: "Invalid pixel data".to_string(),
                    };
                }
            };

        // Maintain aspect ratio
        let aspect = input.height as f64 / input.width as f64;
        let target_height = (input.target_width as f64 * aspect).round() as u32;

        let resized = img.resize_exact(
            input.target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        );

        let rgba = resized.to_rgba8();
        ImageResult::Success {
            image: DecodedImage {
                width: rgba.width(),
                height: rgba.height(),
                pixels: rgba.into_raw(),
                channels: 4,
            },
        }
    }

    async fn generate_thumbhash_data_url(&self, input: ThumbhashInput) -> ImageResult {
        let img = match pixels_to_dynamic_image(&input.pixels, input.width, input.height, 4) {
            Some(img) => img,
            None => {
                return ImageResult::Error {
                    message: "Invalid pixel data".to_string(),
                };
            }
        };

        // Thumbhash works best with small images, resize if needed
        let thumb_img = if input.width > 100 || input.height > 100 {
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
            Err(()) => {
                return ImageResult::Error {
                    message: "Failed to decode thumbhash".to_string(),
                };
            }
        };

        // Create a tiny PNG from the decoded thumbhash
        let img_buf: image::RgbaImage =
            match image::ImageBuffer::from_raw(w as u32, h as u32, rgba_pixels) {
                Some(buf) => buf,
                None => {
                    return ImageResult::Error {
                        message: "Failed to create image buffer".to_string(),
                    };
                }
            };

        let mut png_bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        if let Err(e) = encoder.write_image(
            img_buf.as_raw(),
            img_buf.width(),
            img_buf.height(),
            image::ExtendedColorType::Rgba8,
        ) {
            return ImageResult::Error {
                message: format!("Failed to encode PNG: {e}"),
            };
        }

        // Encode as data URL
        let base64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
        ImageResult::ThumbhashSuccess {
            data_url: format!("data:image/png;base64,{}", base64),
        }
    }
}

fn decode_format(data: &[u8], format: image::ImageFormat) -> ImageResult {
    let img = match image::load_from_memory_with_format(data, format) {
        Ok(img) => img,
        Err(e) => {
            return ImageResult::Error {
                message: format!("Failed to decode image: {e}"),
            };
        }
    };

    let rgba = img.to_rgba8();
    ImageResult::Success {
        image: DecodedImage {
            width: rgba.width(),
            height: rgba.height(),
            pixels: rgba.into_raw(),
            channels: 4,
        },
    }
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

rapace_cell::cell_service!(ImageProcessorServer<ImageProcessorImpl>, ImageProcessorImpl);

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    rapace_cell::run(CellService::from(ImageProcessorImpl)).await?;
    Ok(())
}
