use crate::error::Result;
use captcha_rs::CaptchaBuilder;
use rand::{distributions::Alphanumeric, Rng};

pub struct CaptchaService;

impl CaptchaService {
    pub fn new() -> Self {
        Self
    }

    pub fn generate(
        &self,
        text: Option<String>,
        difficulty: i32,
        width: i32,
        height: i32,
        dark_mode: bool,
        compression: i32,
    ) -> Result<(String, String)> {
        let captcha_text = text.unwrap_or_else(|| Self::generate_random_text(5));

        let captcha = CaptchaBuilder::new()
            .length(captcha_text.len())
            .width(width as u32)
            .height(height as u32)
            .dark_mode(dark_mode)
            .complexity(difficulty as u32)
            .compression(compression as u8)
            .build();

        // to_base64() returns a String directly, not a Result
        let base64_image = captcha.to_base64();

        Ok((captcha_text, base64_image))
    }

    fn generate_random_text(length: usize) -> String {
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(length)
            .map(char::from)
            .collect::<String>()
            .to_uppercase()
    }
}

impl Default for CaptchaService {
    fn default() -> Self {
        Self::new()
    }
}
