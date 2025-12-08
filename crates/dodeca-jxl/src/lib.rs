//! JPEG XL encoding and decoding plugin for dodeca

use facet::Facet;
use jpegxl_rs::encode::EncoderFrame;
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

/// Decode JPEG XL to RGBA pixels
#[plugcard]
pub fn decode_jxl(data: Vec<u8>) -> PlugResult<DecodedImage> {
    let decoder = match jpegxl_rs::decoder_builder().build() {
        Ok(d) => d,
        Err(e) => return PlugResult::Err(format!("Failed to create JXL decoder: {e}")),
    };

    let (metadata, pixels) = match decoder.decode_with::<u8>(&data) {
        Ok(result) => result,
        Err(e) => return PlugResult::Err(format!("Failed to decode JXL: {e}")),
    };

    PlugResult::Ok(DecodedImage {
        pixels,
        width: metadata.width,
        height: metadata.height,
        channels: metadata.num_color_channels as u8 + if metadata.has_alpha_channel { 1 } else { 0 },
    })
}

/// Encode RGBA pixels to JPEG XL
#[plugcard]
pub fn encode_jxl(pixels: Vec<u8>, width: u32, height: u32, quality: u8) -> PlugResult<Vec<u8>> {
    if pixels.len() != (width * height * 4) as usize {
        return PlugResult::Err(format!(
            "Expected {} bytes for {}x{} RGBA, got {}",
            width * height * 4,
            width,
            height,
            pixels.len()
        ));
    }

    // quality 0-100 maps to JXL distance (lower distance = better quality)
    // quality 100 -> distance ~0 (lossless territory)
    // quality 80 -> distance ~2 (high quality)
    // quality 0 -> distance ~15 (low quality)
    let distance = (100.0 - quality as f32) / 100.0 * 15.0;

    let mut encoder = match jpegxl_rs::encoder_builder()
        .quality(distance.max(0.1)) // quality() is actually distance in jpegxl-rs
        .build()
    {
        Ok(e) => e,
        Err(e) => return PlugResult::Err(format!("Failed to create JXL encoder: {e}")),
    };

    encoder.has_alpha = true;
    let frame = EncoderFrame::new(&pixels).num_channels(4);
    let result = match encoder.encode_frame::<_, u8>(&frame, width, height) {
        Ok(r) => r,
        Err(e) => return PlugResult::Err(format!("Failed to encode JXL: {e}")),
    };

    PlugResult::Ok(result.data.to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn debug_struct_size() -> usize {
    std::mem::size_of::<plugcard::MethodSignature>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_jxl() {
        // 16x16 red pixels (RGBA) - JXL encoder needs larger images
        let pixels = [255u8, 0, 0, 255].repeat(16 * 16);

        let result = encode_jxl(pixels, 16, 16, 80);
        let PlugResult::Ok(data) = result else {
            panic!("Expected Ok, got Err");
        };
        assert!(!data.is_empty());
        // JXL magic: 0xff 0x0a for naked codestream
        assert_eq!(&data[0..2], &[0xff, 0x0a]);
    }

    #[test]
    fn test_wrong_size() {
        let pixels = vec![255, 0, 0, 255];
        let result = encode_jxl(pixels, 2, 2, 80);
        assert!(matches!(result, PlugResult::Err(_)));
    }
}
