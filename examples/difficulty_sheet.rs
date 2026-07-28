//! Renders every difficulty level side by side, several independent draws each.
//!
//! The one sheet to look at when judging whether the difficulty dial is
//! reasonable end to end: one row per level 1-10, `draws` columns of
//! independently generated challenges, each with its own random solution.
//!
//! Everything goes through `CaptchaService::generate` at the shipped JPEG
//! quality, so what the sheet shows is what the HTTP API serves — including the
//! compression artifacts, which a prettier test render would hide.
//!
//! ```text
//! cargo run --release --example difficulty_sheet -- <out-dir> [draws] [length]
//! ```
//!
//! Writes `<out-dir>/difficulty-NN-<n>.jpeg` for every tile plus a combined
//! `<out-dir>/by-difficulty.jpeg`, and prints the solutions. Solutions are on
//! stdout deliberately, unlike `challenge_set`: this is an illustration rather
//! than a benchmark, and the useful question — is level 5 still legible to a
//! human? — cannot be answered without knowing the answer.

use captchapi::services::CaptchaService;
use captchapi::validation::{
    DEFAULT_COMPRESSION, DEFAULT_HEIGHT, DEFAULT_LENGTH, DEFAULT_WIDTH, DIFFICULTY_MAX,
    DIFFICULTY_MIN,
};
use image::{DynamicImage, ImageBuffer, Rgb, RgbImage};

/// Gap between tiles, and the colour of it.
const GAP: u32 = 4;
const MOUNT: Rgb<u8> = Rgb([70, 70, 78]);

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| {
        eprintln!("usage: difficulty_sheet <out-dir> [draws] [length]");
        std::process::exit(2);
    });
    let draws: u32 = args
        .next()
        .map(|n| n.parse().expect("draws must be a number"))
        .unwrap_or(3);
    let length: i64 = args
        .next()
        .map(|n| n.parse().expect("length must be a number"))
        .unwrap_or(DEFAULT_LENGTH);

    std::fs::create_dir_all(&dir).expect("output directory is writable");
    let service = CaptchaService::new();

    let levels: Vec<i64> = (DIFFICULTY_MIN..=DIFFICULTY_MAX).collect();
    let (width, height) = (DEFAULT_WIDTH as u32, DEFAULT_HEIGHT as u32);
    let mut sheet: RgbImage = ImageBuffer::from_pixel(
        draws * width + (draws + 1) * GAP,
        levels.len() as u32 * height + (levels.len() as u32 + 1) * GAP,
        MOUNT,
    );

    println!(
        "{draws} draws per level, length {length}, {width}x{height}, \
         quality {DEFAULT_COMPRESSION}\n"
    );
    println!("{:>5}  {:<24} bytes", "level", "solutions");

    for (row, difficulty) in levels.iter().enumerate() {
        let mut solutions = Vec::new();
        let mut sizes = Vec::new();
        for column in 0..draws {
            let (text, bytes) = service
                .generate(
                    length,
                    *difficulty,
                    DEFAULT_WIDTH,
                    DEFAULT_HEIGHT,
                    false,
                    DEFAULT_COMPRESSION,
                )
                .expect("render succeeds");

            let name = format!("difficulty-{difficulty:02}-{}.jpeg", column + 1);
            std::fs::write(format!("{dir}/{name}"), &bytes).expect("tile writes");

            // Decoded back rather than re-rendered, so the tile in the sheet is
            // byte-for-byte the image written beside it, artifacts included.
            let tile = image::load_from_memory(&bytes)
                .expect("the encoder's own output decodes")
                .to_rgb8();
            let ox = GAP + column * (width + GAP);
            let oy = GAP + row as u32 * (height + GAP);
            for (x, y, pixel) in tile.enumerate_pixels() {
                if ox + x < sheet.width() && oy + y < sheet.height() {
                    sheet.put_pixel(ox + x, oy + y, *pixel);
                }
            }

            solutions.push(text);
            sizes.push(bytes.len());
        }
        println!(
            "{difficulty:>5}  {:<24} {}",
            solutions.join(" "),
            sizes
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    let combined = format!("{dir}/by-difficulty.jpeg");
    DynamicImage::ImageRgb8(sheet)
        .to_rgb8()
        .save_with_format(&combined, image::ImageFormat::Jpeg)
        .expect("sheet writes");
    println!(
        "\nwrote {} tiles and {combined}",
        levels.len() as u32 * draws
    );
}
