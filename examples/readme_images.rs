//! Regenerates the two sample images the README shows.
//!
//! They are rendered through the production path, so the README shows what the
//! HTTP API actually serves. Re-run it whenever the renderer changes — a
//! screenshot of an older deformation set is a quietly wrong claim about the
//! service, and the README is the first thing anyone reads.
//!
//! ```text
//! cargo run --release --example readme_images
//! ```
//!
//! Solutions go to stdout here, deliberately unlike `challenge_set`: these are
//! illustrations, not a benchmark, and knowing what they say is useful when
//! judging whether the render is still legible.

use captchapi::services::CaptchaService;
use captchapi::validation::{DEFAULT_COMPRESSION, DEFAULT_HEIGHT, DEFAULT_LENGTH, DEFAULT_WIDTH};

fn main() {
    let service = CaptchaService::new();
    let dir = "docs/images";
    std::fs::create_dir_all(dir).expect("output directory is writable");

    for (name, difficulty, dark) in [
        ("captcha-easy.jpeg", 2i64, false),
        ("captcha-hard-dark.jpeg", 8, true),
    ] {
        let (text, bytes) = service
            .generate(
                DEFAULT_LENGTH,
                difficulty,
                DEFAULT_WIDTH,
                DEFAULT_HEIGHT,
                dark,
                DEFAULT_COMPRESSION,
            )
            .expect("render succeeds");
        std::fs::write(format!("{dir}/{name}"), &bytes).expect("image writes");
        println!(
            "{name}: difficulty {difficulty}, dark {dark}, {} bytes, solution {text:?}",
            bytes.len()
        );
    }
}
