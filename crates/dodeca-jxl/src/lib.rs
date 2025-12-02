//! JPEG XL encoding and decoding plugin for dodeca

use jpegxl_rs::encode::EncoderFrame;
use plugcard::plugcard;

plugcard::export_plugin!();

/// Decoded image data
#[derive(serde::Serialize, serde::Deserialize, postcard_schema::Schema)]
pub struct DecodedImage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub channels: u8,
}

/// Decode JPEG XL to RGBA pixels
#[plugcard]
pub fn decode_jxl(data: Vec<u8>) -> Result<DecodedImage, String> {
    let decoder = jpegxl_rs::decoder_builder()
        .build()
        .map_err(|e| format!("Failed to create JXL decoder: {e}"))?;

    let (metadata, pixels) = decoder
        .decode_with::<u8>(&data)
        .map_err(|e| format!("Failed to decode JXL: {e}"))?;

    Ok(DecodedImage {
        pixels,
        width: metadata.width,
        height: metadata.height,
        channels: metadata.num_color_channels as u8 + if metadata.has_alpha_channel { 1 } else { 0 },
    })
}

/// Encode RGBA pixels to JPEG XL
#[plugcard]
pub fn encode_jxl(pixels: Vec<u8>, width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    if pixels.len() != (width * height * 4) as usize {
        return Err(format!(
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

    let mut encoder = jpegxl_rs::encoder_builder()
        .quality(distance.max(0.1)) // quality() is actually distance in jpegxl-rs
        .build()
        .map_err(|e| format!("Failed to create JXL encoder: {e}"))?;

    encoder.has_alpha = true;
    let frame = EncoderFrame::new(&pixels).num_channels(4);
    let result = encoder
        .encode_frame::<_, u8>(&frame, width, height)
        .map_err(|e| format!("Failed to encode JXL: {e}"))?;

    Ok(result.data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_jxl() {
        // 16x16 red pixels (RGBA) - JXL encoder needs larger images
        let pixels = vec![255u8, 0, 0, 255].repeat(16 * 16);

        let result = encode_jxl(pixels, 16, 16, 80).unwrap();
        assert!(!result.is_empty());
        // JXL magic: 0xff 0x0a for naked codestream
        assert_eq!(&result[0..2], &[0xff, 0x0a]);
    }

    #[test]
    fn test_wrong_size() {
        let pixels = vec![255, 0, 0, 255];
        let result = encode_jxl(pixels, 2, 2, 80);
        assert!(result.is_err());
    }
}
