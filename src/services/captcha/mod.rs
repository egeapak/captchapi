mod generator;

use crate::error::Result;
use image::DynamicImage;

pub struct CaptchaService;

impl CaptchaService {
    pub fn new() -> Self {
        Self
    }

    #[tracing::instrument(
        skip(self),
        fields(
            length,
            difficulty,
            width,
            height,
            dark_mode,
            compression,
            image_size_bytes
        )
    )]
    pub fn generate(
        &self,
        length: i64,
        difficulty: i64,
        width: i64,
        height: i64,
        dark_mode: bool,
        compression: i64,
    ) -> Result<(String, Vec<u8>)> {
        tracing::Span::current().record("length", length);

        let (text, image) = generator::generate(
            length as usize,
            difficulty as u32,
            width as u32,
            height as u32,
            dark_mode,
        );

        // Get raw JPEG bytes from the DynamicImage
        let image_bytes = Self::image_to_jpeg_bytes(&image, compression as u8)?;

        tracing::Span::current().record("image_size_bytes", image_bytes.len());

        // Return the ACTUAL text that was generated
        Ok((text, image_bytes))
    }

    /// Convert a DynamicImage to JPEG bytes with specified quality
    pub fn image_to_jpeg_bytes(image: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
        use image::codecs::jpeg::JpegEncoder;

        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut bytes, quality);

        encoder.encode_image(image).map_err(|e| {
            crate::error::AppError::Internal(anyhow::anyhow!("Failed to encode JPEG: {}", e))
        })?;

        Ok(bytes)
    }
}

impl Default for CaptchaService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_with_custom_length() {
        let service = CaptchaService::new();
        let custom_length = 7;

        let result = service.generate(custom_length, 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, image_bytes) = result.unwrap();
        assert_eq!(
            text.len(),
            custom_length as usize,
            "Generated text should match requested length"
        );
        assert!(!image_bytes.is_empty(), "Image should not be empty");

        // Verify the image bytes are valid JPEG
        let jpeg_signature: [u8; 3] = [255, 216, 255]; // 0xFF 0xD8 0xFF
        assert!(
            image_bytes.len() >= 3,
            "Image data should be at least 3 bytes (JPEG signature size)"
        );

        assert!(
            image_bytes.starts_with(&jpeg_signature),
            "Image data should start with JPEG signature (0xFF 0xD8 0xFF). Got: {:?}",
            &image_bytes[..8.min(image_bytes.len())]
        );

        // Verify we have substantial image data (not just header)
        assert!(
            image_bytes.len() > 1000,
            "JPEG should have substantial data, got {} bytes",
            image_bytes.len()
        );
    }

    #[test]
    fn test_generate_with_default_length() {
        let service = CaptchaService::new();

        let result = service.generate(5, 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, image_bytes) = result.unwrap();
        assert_eq!(text.len(), 5, "Generated text should be 5 characters");
        assert!(
            text.chars().all(|c| c.is_alphanumeric()),
            "Should be alphanumeric (uppercase or lowercase)"
        );
        assert!(!image_bytes.is_empty(), "Image should not be empty");

        // Verify JPEG signature
        let jpeg_signature: [u8; 3] = [255, 216, 255];
        assert!(
            image_bytes.starts_with(&jpeg_signature),
            "Should be a valid JPEG file"
        );
        assert!(
            image_bytes.len() > 1000,
            "JPEG should have substantial data"
        );
    }

    #[test]
    fn test_generate_with_different_parameters() {
        let service = CaptchaService::new();

        let result1 = service.generate(3, 1, 100, 50, false, 40);
        let result2 = service.generate(3, 10, 300, 150, true, 40);

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let (text1, image1) = result1.unwrap();
        let (text2, image2) = result2.unwrap();

        // Both should have the same length
        assert_eq!(text1.len(), 3);
        assert_eq!(text2.len(), 3);

        // Different parameters should produce different images
        assert_ne!(image1, image2);
    }

    #[test]
    fn test_generate_returns_jpeg_bytes() {
        let service = CaptchaService::new();

        let result = service.generate(4, 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, bytes) = result.unwrap();
        assert_eq!(text.len(), 4, "Generated text should be 4 characters");

        // Verify JPEG signature
        let jpeg_signature: [u8; 3] = [255, 216, 255];
        assert!(
            bytes.starts_with(&jpeg_signature),
            "Should be valid JPEG file"
        );
        assert!(bytes.len() > 1000, "JPEG should have substantial data");
    }

    #[test]
    fn test_default_creates_service() {
        let service = CaptchaService;
        // Verify the default instance works just like one created with new()
        let result = service.generate(5, 5, 220, 120, false, 40);
        assert!(result.is_ok());
        let (text, image_bytes) = result.unwrap();
        assert_eq!(text.len(), 5);
        assert!(!image_bytes.is_empty());
    }

    #[test]
    fn test_image_to_jpeg_bytes() {
        // Create a simple test image
        use image::{ImageBuffer, Rgb};
        let img = ImageBuffer::from_fn(100, 100, |x, y| {
            if (x + y) % 2 == 0 {
                Rgb([255u8, 0, 0])
            } else {
                Rgb([0u8, 0, 255])
            }
        });
        let dynamic_img = image::DynamicImage::ImageRgb8(img);

        let result = CaptchaService::image_to_jpeg_bytes(&dynamic_img, 80);

        assert!(result.is_ok());
        let bytes = result.unwrap();

        // Verify JPEG signature
        let jpeg_signature: [u8; 3] = [255, 216, 255];
        assert!(bytes.starts_with(&jpeg_signature), "Should be JPEG");
        assert!(!bytes.is_empty());
    }
}
