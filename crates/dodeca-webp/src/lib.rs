//! WebP encoding and decoding plugin for dodeca

use facet::Facet;
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

/// Decode WebP to RGBA/RGB pixels
#[plugcard]
pub fn decode_webp(data: Vec<u8>) -> PlugResult<DecodedImage> {
    let decoder = webp::Decoder::new(&data);
    let image = match decoder.decode() {
        Some(img) => img,
        None => return PlugResult::Err("Failed to decode WebP".to_string()),
    };

    PlugResult::Ok(DecodedImage {
        pixels: (*image).to_vec(),
        width: image.width(),
        height: image.height(),
        channels: if image.is_alpha() { 4 } else { 3 },
    })
}

/// Encode RGBA pixels to WebP
#[plugcard]
pub fn encode_webp(pixels: Vec<u8>, width: u32, height: u32, quality: u8) -> PlugResult<Vec<u8>> {
    if pixels.len() != (width * height * 4) as usize {
        return PlugResult::Err(format!(
            "Expected {} bytes for {}x{} RGBA, got {}",
            width * height * 4,
            width,
            height,
            pixels.len()
        ));
    }

    let encoder = webp::Encoder::from_rgba(&pixels, width, height);
    let webp = encoder.encode(quality as f32);

    PlugResult::Ok(webp.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_webp() {
        // 2x2 red pixels (RGBA)
        let pixels = vec![
            255, 0, 0, 255,  // red
            255, 0, 0, 255,  // red
            255, 0, 0, 255,  // red
            255, 0, 0, 255,  // red
        ];

        let result = encode_webp(pixels, 2, 2, 80);
        let PlugResult::Ok(data) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(!data.is_empty());
        assert_eq!(&data[0..4], b"RIFF");
    }

    #[test]
    fn test_wrong_size() {
        let pixels = vec![255, 0, 0, 255]; // 1 pixel
        let result = encode_webp(pixels, 2, 2, 80); // claims 2x2
        assert!(matches!(result, PlugResult::Err(_)));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}
