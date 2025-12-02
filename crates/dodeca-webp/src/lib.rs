//! WebP encoding and decoding plugin for dodeca

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

/// Decode WebP to RGBA/RGB pixels
#[plugcard]
pub fn decode_webp(data: Vec<u8>) -> Result<DecodedImage, String> {
    let decoder = webp::Decoder::new(&data);
    let image = decoder.decode().ok_or("Failed to decode WebP")?;

    Ok(DecodedImage {
        pixels: (*image).to_vec(),
        width: image.width(),
        height: image.height(),
        channels: if image.is_alpha() { 4 } else { 3 },
    })
}

/// Encode RGBA pixels to WebP
#[plugcard]
pub fn encode_webp(pixels: Vec<u8>, width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    if pixels.len() != (width * height * 4) as usize {
        return Err(format!(
            "Expected {} bytes for {}x{} RGBA, got {}",
            width * height * 4,
            width,
            height,
            pixels.len()
        ));
    }

    let encoder = webp::Encoder::from_rgba(&pixels, width, height);
    let webp = encoder.encode(quality as f32);

    Ok(webp.to_vec())
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

        let result = encode_webp(pixels, 2, 2, 80).unwrap();
        assert!(!result.is_empty());
        assert_eq!(&result[0..4], b"RIFF");
    }

    #[test]
    fn test_wrong_size() {
        let pixels = vec![255, 0, 0, 255]; // 1 pixel
        let result = encode_webp(pixels, 2, 2, 80); // claims 2x2
        assert!(result.is_err());
    }
}
