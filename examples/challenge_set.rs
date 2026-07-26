//! Writes a labelled set of CAPTCHA challenges for solver evaluation.
//!
//! Renders through the production path — `CaptchaService::generate`, at the
//! shipped JPEG quality — so a solve rate measured here is a solve rate against
//! what the HTTP API actually serves, not against a prettier test render.
//!
//! ```text
//! cargo run --release --example challenge_set -- <out-dir> [draws-per-cell]
//! ```
//!
//! Produces `<out-dir>/NNN.jpg` plus `<out-dir>/manifest.json` mapping each
//! file to its solution, length and difficulty. **The solutions are written to
//! the manifest only, never to stdout**, so an operator can look at the images
//! and attempt them before revealing the answers — reading a solution before
//! guessing makes the measurement worthless, and stdout is hard to unsee.

use captchapi::services::CaptchaService;
use captchapi::validation::{DEFAULT_COMPRESSION, DEFAULT_HEIGHT, DEFAULT_WIDTH};

/// The grid. Lengths and difficulties are crossed, so every cell is one
/// (length, difficulty) pair drawn `draws` times.
const LENGTHS: [i64; 3] = [4, 5, 6];
const DIFFICULTIES: [i64; 4] = [3, 5, 8, 10];

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| {
        eprintln!("usage: challenge_set <out-dir> [draws-per-cell]");
        std::process::exit(2);
    });
    let draws: usize = args
        .next()
        .map(|n| n.parse().expect("draws-per-cell must be a number"))
        .unwrap_or(1);

    std::fs::create_dir_all(&dir).expect("output directory is writable");

    let service = CaptchaService::new();
    let mut manifest = Vec::new();
    let mut index = 0;

    for difficulty in DIFFICULTIES {
        for length in LENGTHS {
            for _ in 0..draws {
                let (solution, jpeg) = service
                    .generate(
                        length,
                        difficulty,
                        DEFAULT_WIDTH,
                        DEFAULT_HEIGHT,
                        false,
                        DEFAULT_COMPRESSION,
                    )
                    .expect("render succeeds");

                let name = format!("{index:03}.jpg");
                std::fs::write(format!("{dir}/{name}"), &jpeg).expect("image writes");
                manifest.push(format!(
                    "  {{\"file\": \"{name}\", \"solution\": \"{solution}\", \
                     \"length\": {length}, \"difficulty\": {difficulty}, \
                     \"bytes\": {}}}",
                    jpeg.len()
                ));
                index += 1;
            }
        }
    }

    std::fs::write(
        format!("{dir}/manifest.json"),
        format!("[\n{}\n]\n", manifest.join(",\n")),
    )
    .expect("manifest writes");

    // Deliberately no solutions here.
    println!(
        "wrote {index} challenges to {dir}/ ({} per cell, lengths {LENGTHS:?}, \
         difficulties {DIFFICULTIES:?}) at {DEFAULT_WIDTH}x{DEFAULT_HEIGHT} quality \
         {DEFAULT_COMPRESSION}",
        draws
    );
}
