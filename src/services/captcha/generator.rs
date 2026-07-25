//! CAPTCHA image generation.
//!
//! Derived from the `captcha-rs` crate (v0.5.0), MIT licensed,
//! Copyright (c) 2022 Samir Djelal — see THIRD_PARTY_LICENSES.
//!
//! Vendored so that CaptchAPI controls the `image`/`imageproc` feature set.
//! Upstream pulls in `image/default`, which drags every codec (AVIF, EXR,
//! TIFF, PNG, WebP, ...) into the build even though only JPEG is used.
//! Upstream also embeds a proprietary Monotype Arial; this port uses
//! Liberation Sans Bold (SIL OFL 1.1), which is metric-compatible.

use ab_glyph::FontArc;
use image::{DynamicImage, ImageBuffer, Rgb};
use imageproc::drawing::{draw_cubic_bezier_curve_mut, draw_hollow_ellipse_mut, draw_text_mut};
use imageproc::noise::{gaussian_noise_mut, salt_and_pepper_noise_mut};
use rand::{rng, RngExt};
use std::sync::OnceLock;

/// Character set for generated solutions.
///
/// Excludes 0, O, I, l and other glyphs that are easily confused.
const BASIC_CHAR: [char; 54] = [
    '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'M',
    'N', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f', 'g',
    'h', 'j', 'k', 'm', 'n', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

/// Background colours.
const LIGHT: [u8; 3] = [224, 238, 253];
const DARK: [u8; 3] = [18, 18, 18];

/// Glyph colours, picked at random per character.
const LIGHT_BASIC_COLOR: [[u8; 3]; 5] = [
    [214, 14, 50],
    [240, 181, 41],
    [176, 203, 40],
    [105, 137, 194],
    [242, 140, 71],
];
const DARK_BASIC_COLOR: [[u8; 3]; 5] = [
    [251, 188, 5],
    [116, 192, 255],
    [255, 224, 133],
    [198, 215, 97],
    [247, 185, 168],
];

/// Font sizes, selected by solution length.
const SCALE_SM: f32 = 35.0;
const SCALE_MD: f32 = 42.0;
const SCALE_LG: f32 = 50.0;

/// Number of interference curves and ellipses drawn over the glyphs.
const INTERFERENCE_LINES: usize = 2;
const INTERFERENCE_ELLIPSES: usize = 2;

/// Liberation Sans Bold, SIL OFL 1.1 — see assets/fonts/.
static FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

/// Parsed once per process; upstream re-parsed the 400 KB face for every
/// character of every CAPTCHA.
fn font() -> &'static FontArc {
    static FONT: OnceLock<FontArc> = OnceLock::new();
    FONT.get_or_init(|| {
        FontArc::try_from_slice(FONT_BYTES).expect("bundled Liberation Sans Bold is a valid TTF")
    })
}

/// Random number in `0..=num`.
fn get_rnd(num: usize) -> usize {
    rng().random_range(0..=num)
}

/// Random float in `min..=max`, saturating when the range is empty.
fn get_next(min: f32, max: u32) -> f32 {
    if (max as f32) <= min {
        return min;
    }
    min + get_rnd(max as usize - min as usize) as f32
}

/// Build a random solution string of `len` characters.
fn random_text(len: usize) -> String {
    let max_idx = BASIC_CHAR.len() - 1;
    (0..len).map(|_| BASIC_CHAR[get_rnd(max_idx)]).collect()
}

/// Random glyph colour for the current mode.
fn get_color(dark_mode: bool) -> Rgb<u8> {
    let rnd = get_rnd(4);
    if dark_mode {
        Rgb(DARK_BASIC_COLOR[rnd])
    } else {
        Rgb(LIGHT_BASIC_COLOR[rnd])
    }
}

/// Background canvas.
fn background(width: u32, height: u32, dark_mode: bool) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let fill = if dark_mode { DARK } else { LIGHT };
    ImageBuffer::from_fn(width, height, |_, _| Rgb(fill))
}

/// Lay the solution characters out across the canvas.
fn write_characters(text: &str, image: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, dark_mode: bool) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return;
    }

    let usable_width = image.width().saturating_sub(10);
    let step = usable_width / chars.len() as u32;
    let y = (image.height() / 2).saturating_sub(15) as i32;

    let scale = match chars.len() {
        1..=3 => SCALE_LG,
        4..=5 => SCALE_MD,
        _ => SCALE_SM,
    };

    let font = font();
    for (i, ch) in chars.iter().enumerate() {
        let x = 5 + (i as u32 * step) as i32;
        let mut buf = [0u8; 4];
        draw_text_mut(
            image,
            get_color(dark_mode),
            x,
            y,
            scale,
            font,
            ch.encode_utf8(&mut buf),
        );
    }
}

/// Draw a random bezier curve across the image.
fn draw_interference_line(image: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, dark_mode: bool) {
    let (width, height) = (image.width(), image.height());
    if width <= 5 || height <= 5 {
        return;
    }

    let x1: f32 = 5.0;
    let y1 = get_next(x1, height / 2);
    let x2 = width.saturating_sub(5) as f32;
    let y2 = get_next((height / 2) as f32, height.saturating_sub(5));

    let ctrl_x = get_next((width / 4) as f32, width / 4 * 3);
    let ctrl_y = get_next(x1, height - 5);
    let ctrl_x2 = get_next((width / 4) as f32, width / 4 * 3);
    let ctrl_y2 = get_next(x1, height - 5);

    draw_cubic_bezier_curve_mut(
        image,
        (x1, y1),
        (x2, y2),
        (ctrl_x, ctrl_y),
        (ctrl_x2, ctrl_y2),
        get_color(dark_mode),
    );
}

/// Scatter hollow ellipses over the image.
fn draw_interference_ellipses(
    count: usize,
    image: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    dark_mode: bool,
) {
    if image.width() <= 25 || image.height() <= 15 {
        return;
    }
    for _ in 0..count {
        let radius = (10 + get_rnd(5)) as i32;
        let x = get_rnd((image.width() - 25) as usize) as i32;
        let y = get_rnd((image.height() - 15) as usize) as i32;
        draw_hollow_ellipse_mut(image, (x, y), radius, radius, get_color(dark_mode));
    }
}

/// Render a CAPTCHA, returning the solution text and its image.
///
/// `difficulty` (1-10) controls how much noise is layered on top.
pub fn generate(
    length: usize,
    difficulty: u32,
    width: u32,
    height: u32,
    dark_mode: bool,
) -> (String, DynamicImage) {
    let length = length.clamp(1, 32);
    let difficulty = difficulty.clamp(1, 10);
    let width = width.clamp(30, 2000);
    let height = height.clamp(20, 2000);

    let text = random_text(length);
    let mut image = background(width, height, dark_mode);

    write_characters(&text, &mut image, dark_mode);

    for _ in 0..INTERFERENCE_LINES {
        draw_interference_line(&mut image, dark_mode);
    }
    draw_interference_ellipses(INTERFERENCE_ELLIPSES, &mut image, dark_mode);

    if difficulty > 1 {
        let mut rng = rng();
        gaussian_noise_mut(
            &mut image,
            (difficulty - 1) as f64,
            ((5 * difficulty) - 5) as f64,
            rng.random::<u64>(),
        );
        salt_and_pepper_noise_mut(
            &mut image,
            (0.002 * difficulty as f64) - 0.002,
            rng.random::<u64>(),
        );
    }

    (text, DynamicImage::ImageRgb8(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_parses_and_is_cached() {
        let a = font();
        let b = font();
        assert!(std::ptr::eq(a, b), "font should be parsed once");
    }

    #[test]
    fn test_generate_returns_requested_length() {
        let (text, image) = generate(7, 5, 220, 120, false);
        assert_eq!(text.chars().count(), 7);
        assert_eq!(image.width(), 220);
        assert_eq!(image.height(), 120);
    }

    #[test]
    fn test_generate_uses_only_unambiguous_characters() {
        let (text, _) = generate(20, 5, 220, 120, false);
        assert!(
            text.chars().all(|c| BASIC_CHAR.contains(&c)),
            "unexpected characters in {text}"
        );
    }

    #[test]
    fn test_generate_clamps_out_of_range_input() {
        let (text, image) = generate(0, 99, 1, 1, false);
        assert_eq!(text.chars().count(), 1, "length clamps up to 1");
        assert_eq!(image.width(), 30, "width clamps up to the minimum");
        assert_eq!(image.height(), 20, "height clamps up to the minimum");
    }

    #[test]
    fn test_dark_mode_changes_background() {
        let light = background(4, 4, false);
        let dark = background(4, 4, true);
        assert_eq!(light.get_pixel(0, 0), &Rgb(LIGHT));
        assert_eq!(dark.get_pixel(0, 0), &Rgb(DARK));
    }

    #[test]
    fn test_difficulty_one_skips_noise() {
        // With no noise the background corners stay untouched by glyphs,
        // which sit in the middle of a wide canvas.
        let (_, image) = generate(1, 1, 400, 200, false);
        assert_eq!(image.to_rgb8().get_pixel(399, 0), &Rgb(LIGHT));
    }

    #[test]
    fn test_generates_distinct_solutions() {
        let solutions: std::collections::HashSet<String> =
            (0..16).map(|_| generate(6, 5, 220, 120, false).0).collect();
        assert!(solutions.len() > 1, "solutions should not be constant");
    }

    #[test]
    fn test_tiny_canvas_does_not_panic() {
        for (w, h) in [(30, 20), (31, 21), (50, 30)] {
            let (_, image) = generate(5, 10, w, h, true);
            assert_eq!(image.width(), w);
        }
    }
}
