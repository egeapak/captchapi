use crate::error::Result;
use captcha_rs::CaptchaBuilder;
use image::DynamicImage;
use rand::{distributions::Alphanumeric, Rng};
use std::io::Cursor;

pub struct CaptchaService;

impl CaptchaService {
    pub fn new() -> Self {
        Self
    }

    pub fn generate(
        &self,
        text: Option<String>,
        difficulty: i64,
        width: i64,
        height: i64,
        dark_mode: bool,
        compression: i64,
    ) -> Result<(String, Vec<u8>)> {
        let captcha_text = text.unwrap_or_else(|| Self::generate_random_text(5));

        let captcha = CaptchaBuilder::new()
            .length(captcha_text.len())
            .width(width as u32)
            .height(height as u32)
            .dark_mode(dark_mode)
            .complexity(difficulty as u32)
            .compression(compression as u8)
            .build();

        // Get raw JPEG bytes from the DynamicImage
        let image_bytes = Self::image_to_jpeg_bytes(&captcha.image, compression as u8)?;

        Ok((captcha_text, image_bytes))
    }

    fn generate_random_text(length: usize) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect::<String>()
    }

    /// Convert a DynamicImage to JPEG bytes
    pub fn image_to_jpeg_bytes(image: &DynamicImage, _quality: u8) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut cursor = Cursor::new(&mut bytes);

        image
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .map_err(|e| {
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
    fn test_generate_with_custom_text() {
        let service = CaptchaService::new();
        let custom_text = "TEST123";

        let result = service.generate(Some(custom_text.to_string()), 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, image_bytes) = result.unwrap();
        assert_eq!(text, custom_text);
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
    fn test_generate_with_random_text() {
        let service = CaptchaService::new();

        let result = service.generate(None, 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, image_bytes) = result.unwrap();
        assert_eq!(text.len(), 5, "Random text should be 5 characters");
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

        let result1 = service.generate(Some("ABC".to_string()), 1, 100, 50, false, 40);
        let result2 = service.generate(Some("ABC".to_string()), 10, 300, 150, true, 40);

        assert!(result1.is_ok());
        assert!(result2.is_ok());

        let (_, image1) = result1.unwrap();
        let (_, image2) = result2.unwrap();

        // Different parameters should produce different images
        assert_ne!(image1, image2);
    }

    #[test]
    fn test_generate_random_text_length() {
        let text1 = CaptchaService::generate_random_text(5);
        let text2 = CaptchaService::generate_random_text(10);

        assert_eq!(text1.len(), 5);
        assert_eq!(text2.len(), 10);
    }

    #[test]
    fn test_generate_random_text_is_random() {
        let text1 = CaptchaService::generate_random_text(8);
        let text2 = CaptchaService::generate_random_text(8);

        // Should be extremely unlikely to be the same
        assert_ne!(text1, text2);
    }

    #[test]
    fn test_generate_returns_jpeg_bytes() {
        let service = CaptchaService::new();

        let result = service.generate(Some("TEST".to_string()), 5, 220, 120, false, 40);

        assert!(result.is_ok());
        let (text, bytes) = result.unwrap();
        assert_eq!(text, "TEST");

        // Verify JPEG signature
        let jpeg_signature: [u8; 3] = [255, 216, 255];
        assert!(
            bytes.starts_with(&jpeg_signature),
            "Should be valid JPEG file"
        );
        assert!(bytes.len() > 1000, "JPEG should have substantial data");
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
