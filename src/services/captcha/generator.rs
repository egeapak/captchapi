//! CAPTCHA image generation.
//!
//! Derived from the `captcha-rs` crate (v0.5.0), MIT licensed,
//! Copyright (c) 2022 Samir Djelal — see THIRD_PARTY_LICENSES.
//!
//! Vendored so that CaptchAPI controls the `image` feature set. Upstream pulls
//! in `image/default`, which drags every codec (AVIF, EXR, TIFF, PNG, WebP,
//! ...) into the build even though only JPEG is used. Upstream also embeds a
//! proprietary Monotype Arial; this port uses a subset of Roboto Bold
//! (SIL OFL 1.1, no Reserved Font Name).
//!
//! The drawing and noise routines this calls are vendored too, in
//! [`super::drawing`] — `imageproc` is no longer a dependency at all.

use super::drawing::{
    composite_mask, draw_cubic_bezier_curve_mut, draw_hollow_circle_mut, gaussian_noise_mut,
    rasterize_char, salt_and_pepper_noise_mut,
};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{DynamicImage, ImageBuffer, Rgb};
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

/// At full intensity, a letter shifts by up to this fraction of its font size,
/// independently in x and y.
const MAX_JITTER: f32 = 0.16;

/// At full intensity, a letter's width and height each vary by up to this
/// fraction, drawn independently so letters stretch as well as grow.
const MAX_SCALE_VARIANCE: f32 = 0.30;

/// At full intensity, a letter leans by up to this shear factor — the
/// horizontal shift per pixel of height, so 0.40 is roughly 22 degrees.
const MAX_SKEW: f32 = 0.40;

/// At full intensity, the sine wave pushes a row sideways by up to this
/// fraction of the font size.
const MAX_WAVE_AMPLITUDE: f32 = 0.14;

/// The wave's period, as a multiple of the letter's height. Under 1.0 a letter
/// shows more than a full cycle, which reads as a wobble rather than a bend.
const WAVE_PERIOD: std::ops::Range<f32> = 0.7..1.6;

/// How strongly each per-letter deformation is applied.
///
/// Every field is an intensity in `0.0..=1.0`, and `0.0` skips that
/// deformation entirely — [`Deformations::none()`] renders exactly what the
/// renderer produced before any of this existed, which is what lets the
/// existing output tests keep their meaning.
///
/// These are runtime values rather than cargo features on purpose: the
/// intensities are driven by a per-request difficulty, so a compile-time
/// switch could not express them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Deformations {
    /// Random displacement of each letter from its laid-out position.
    pub jitter: f32,
    /// Random per-letter scale, drawn separately for width and height.
    pub scale: f32,
    /// Random per-letter lean, as a horizontal shear about the letter's middle.
    pub skew: f32,
    /// Sine displacement down each letter, at a random amplitude and phase.
    pub wave: f32,
}

impl Deformations {
    /// No deformation — the pre-existing rendering.
    pub const fn none() -> Self {
        Self {
            jitter: 0.0,
            scale: 0.0,
            skew: 0.0,
            wave: 0.0,
        }
    }

    /// The intensities a difficulty level implies.
    ///
    /// Linear from nothing at the easiest level to full intensity at the
    /// hardest, which is how the rest of the renderer already reads the level:
    /// both noise generators are scaled by `difficulty - 1`, so they contribute
    /// nothing at the bottom of the range either. An easy CAPTCHA therefore
    /// stays upright and evenly spaced, and the deformations arrive together
    /// with the noise rather than on their own schedule.
    ///
    /// Each deformation has its own cap — `MAX_JITTER`, `MAX_SCALE_VARIANCE`,
    /// `MAX_SKEW`, `MAX_WAVE_AMPLITUDE` — so retuning how strong one gets at a
    /// given level is a change to that constant, not to this ramp.
    pub fn for_difficulty(difficulty: u32) -> Self {
        let intensity = (difficulty.clamp(1, 10) - 1) as f32 / 9.0;
        if intensity == 0.0 {
            return Self::none();
        }
        Self {
            jitter: intensity,
            scale: intensity,
            skew: intensity,
            wave: intensity,
        }
    }
}

/// Roboto Bold subset to exactly [`BASIC_CHAR`], SIL OFL 1.1 — see
/// assets/fonts/ and scripts/subset-font.py.
///
/// The subset carries only the glyphs below. Extending `BASIC_CHAR` without
/// rerunning the script renders the new characters as .notdef;
/// `test_every_basic_char_has_a_glyph` catches that.
static FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/Roboto-Bold-subset.ttf");

/// Parsed once per process; upstream re-parsed the whole face for every
/// character of every CAPTCHA.
fn font() -> &'static FontArc {
    static FONT: OnceLock<FontArc> = OnceLock::new();
    FONT.get_or_init(|| {
        FontArc::try_from_slice(FONT_BYTES).expect("bundled glyph subset is a valid TTF")
    })
}

/// Random number in `0..=num`.
fn get_rnd(num: usize) -> usize {
    rng().random_range(0..=num)
}

/// Uniform random in `-magnitude..magnitude`, exactly zero when disabled.
///
/// The zero check is what makes an intensity of `0.0` a true skip rather than
/// a very small deformation, and it keeps the rng untouched so disabling one
/// deformation does not shift the others' random draws.
fn spread(magnitude: f32) -> f32 {
    if magnitude <= 0.0 {
        return 0.0;
    }
    rng().random_range(-magnitude..magnitude)
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
fn write_characters(
    text: &str,
    image: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
    dark_mode: bool,
    deform: Deformations,
) {
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
    let nominal_ascent = font.as_scaled(PxScale::from(scale)).ascent();

    for (i, ch) in chars.iter().enumerate() {
        let x = 5 + (i as u32 * step) as i32;
        // Drawn unconditionally so the colour draw count does not depend on
        // whether a glyph happens to be outlined.
        let color = get_color(dark_mode);

        // Scale is applied when the outline is rasterized, not by resampling a
        // finished bitmap, so a stretched letter stays as crisp as a plain one.
        let px = PxScale {
            x: scale * (1.0 + spread(MAX_SCALE_VARIANCE * deform.scale)),
            y: scale * (1.0 + spread(MAX_SCALE_VARIANCE * deform.scale)),
        };

        // A larger vertical scale pushes the glyph's ascent down, which would
        // slide big letters toward the bottom of the canvas. Compensating by
        // the ascent difference pins the baseline, so letters grow about a
        // shared line instead of drifting.
        let baseline_shift = (font.as_scaled(px).ascent() - nominal_ascent).round() as i32;

        let dx = spread(MAX_JITTER * scale * deform.jitter).round() as i32;
        let dy = spread(MAX_JITTER * scale * deform.jitter).round() as i32;

        // Drawn outside the `if let` for the same reason as the colour: the
        // number of random draws must not depend on whether this particular
        // glyph turned out to have an outline.
        let lean = spread(MAX_SKEW * deform.skew);
        let amplitude = spread(MAX_WAVE_AMPLITUDE * scale * deform.wave);
        let (period, phase) = if deform.wave > 0.0 {
            (
                rng().random_range(WAVE_PERIOD),
                rng().random_range(0.0..std::f32::consts::TAU),
            )
        } else {
            (1.0, 0.0)
        };

        if let Some(mask) = rasterize_char(font, *ch, px) {
            let mask = if lean != 0.0 || amplitude != 0.0 {
                // Both deformations are horizontal displacements that depend
                // only on the row, so they sum into one closure and cost a
                // single resample. Applying them in sequence would filter the
                // glyph twice and soften it for no reason.
                //
                // The shear is taken about the letter's middle so it leans in
                // place, and the wave's period scales with the letter's height
                // so a tall glyph is not cut into more cycles than a short one.
                let centre = mask.height as f32 / 2.0;
                let wavelength = (mask.height as f32 * period).max(1.0);
                mask.displace_rows(|row| {
                    lean * (row - centre)
                        + amplitude * (std::f32::consts::TAU * row / wavelength + phase).sin()
                })
            } else {
                mask
            };
            composite_mask(image, &mask, x + dx, y + dy - baseline_shift, color);
        }
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

/// Scatter hollow circles over the image.
///
/// Upstream asked for ellipses, but always with equal radii — which
/// `imageproc` dispatched straight to its circle routine — so this calls the
/// circle path directly. The name is kept for continuity with `captcha-rs`.
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
        draw_hollow_circle_mut(image, (x, y), radius, get_color(dark_mode));
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
    let text = random_text(length.clamp(1, 32));
    let image = render(
        &text,
        difficulty,
        width,
        height,
        dark_mode,
        Deformations::for_difficulty(difficulty),
    );
    (text, image)
}

/// Renders `text` onto a fresh canvas with the given deformations.
///
/// Split out from [`generate`] so a caller can hold the string fixed. Comparing
/// two deformation intensities is meaningless if the letters change in between.
pub fn render(
    text: &str,
    difficulty: u32,
    width: u32,
    height: u32,
    dark_mode: bool,
    deform: Deformations,
) -> DynamicImage {
    let difficulty = difficulty.clamp(1, 10);
    let width = width.clamp(30, 2000);
    let height = height.clamp(20, 2000);

    let mut image = background(width, height, dark_mode);

    write_characters(text, &mut image, dark_mode, deform);

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

    DynamicImage::ImageRgb8(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeSet;

    const SAMPLE_TEXT: &str = "Kb7mQ";
    const SAMPLE_W: u32 = 220;
    const SAMPLE_H: u32 = 120;

    fn paste(
        sheet: &mut ImageBuffer<Rgb<u8>, Vec<u8>>,
        tile: &ImageBuffer<Rgb<u8>, Vec<u8>>,
        ox: u32,
        oy: u32,
    ) {
        for (x, y, pixel) in tile.enumerate_pixels() {
            sheet.put_pixel(ox + x, oy + y, *pixel);
        }
    }

    /// One row per intensity, `cols` independent draws per row, so both the
    /// strength and the spread of the randomness are visible at a glance.
    fn contact_sheet(rows: &[(u32, Deformations)], cols: u32) -> DynamicImage {
        let gap = 4;
        let width = cols * SAMPLE_W + (cols + 1) * gap;
        let height = rows.len() as u32 * SAMPLE_H + (rows.len() as u32 + 1) * gap;
        let mut sheet = ImageBuffer::from_pixel(width, height, Rgb([70, 70, 78]));

        for (r, (difficulty, deform)) in rows.iter().enumerate() {
            for c in 0..cols {
                let tile =
                    render(SAMPLE_TEXT, *difficulty, SAMPLE_W, SAMPLE_H, false, *deform).to_rgb8();
                let ox = gap + c * (SAMPLE_W + gap);
                let oy = gap + r as u32 * (SAMPLE_H + gap);
                paste(&mut sheet, &tile, ox, oy);
            }
        }
        DynamicImage::ImageRgb8(sheet)
    }

    fn write_jpeg(image: &DynamicImage, name: &str) {
        use image::codecs::jpeg::JpegEncoder;
        let dir = std::env::var("CAPTCHA_SAMPLE_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 92)
            .encode_image(image)
            .expect("sample encodes");
        std::fs::write(format!("{dir}/{name}"), bytes).expect("sample writes");
    }

    /// Renders contact sheets for visual review. Not an assertion, so it is
    /// ignored by default:
    ///
    /// ```text
    /// CAPTCHA_SAMPLE_DIR=/tmp/samples \
    ///   cargo nextest run -E 'test(visual_samples)' --run-ignored all
    /// ```
    #[test]
    #[ignore = "writes sample images for human review, asserts nothing"]
    fn visual_samples() {
        let levels = [0.0, 0.25, 0.5, 0.75, 1.0];

        let jitter: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        jitter: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&jitter, 3), "01-offset-jitter.jpg");

        let scale: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        scale: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&scale, 3), "02-scale-variance.jpg");

        let both: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        jitter: *i,
                        scale: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&both, 3), "03-offset-and-scale.jpg");

        let skew: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        skew: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&skew, 3), "04-skew.jpg");

        let wave: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        wave: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&wave, 3), "05-sine-wave.jpg");

        // Everything on, at the difficulty levels a caller actually asks for,
        // so the noise and the deformations ramp together.
        let by_difficulty: Vec<_> = [1u32, 3, 5, 7, 10]
            .iter()
            .map(|d| (*d, Deformations::for_difficulty(*d)))
            .collect();
        write_jpeg(&contact_sheet(&by_difficulty, 3), "06-by-difficulty.jpg");
    }

    /// Letters only, on a bare canvas — no interference lines, ellipses or
    /// noise — so a deformation can be observed without random clutter on top.
    fn glyph_pixels(text: &str, deform: Deformations) -> BTreeSet<(u32, u32)> {
        let mut image = background(220, 120, false);
        write_characters(text, &mut image, false, deform);
        image
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0 != LIGHT)
            .map(|(x, y, _)| (x, y))
            .collect()
    }

    /// (left, top, right, bottom) of the inked pixels.
    fn bbox(pixels: &BTreeSet<(u32, u32)>) -> (u32, u32, u32, u32) {
        let left = pixels.iter().map(|p| p.0).min().unwrap();
        let right = pixels.iter().map(|p| p.0).max().unwrap();
        let top = pixels.iter().map(|p| p.1).min().unwrap();
        let bottom = pixels.iter().map(|p| p.1).max().unwrap();
        (left, top, right, bottom)
    }

    #[test]
    fn test_spread_is_a_true_skip_when_disabled_and_bounded_otherwise() {
        for _ in 0..500 {
            assert_eq!(spread(0.0), 0.0, "zero intensity must not deform");
            assert_eq!(spread(-3.0), 0.0, "a negative magnitude must not deform");
            let value = spread(5.0);
            assert!((-5.0..5.0).contains(&value), "{value} escaped its bound");
        }
    }

    /// Placement is compared by bounding box with a one-pixel tolerance rather
    /// than by exact pixel set, because the faintest antialiased edge pixels
    /// are colour-dependent: `weighted_sum` truncates, so `224 + 16*gv` floors
    /// back to the background for a low coverage under one glyph colour but
    /// not under another, and the colour is drawn at random. A pixel-exact
    /// comparison would flake on that, not on placement.
    #[test]
    fn test_no_deformation_places_letters_identically_every_time() {
        let first = glyph_pixels("Kb7mQ", Deformations::none());
        assert!(!first.is_empty(), "the letters should have drawn something");
        let (l0, t0, r0, b0) = bbox(&first);

        for _ in 0..8 {
            let again = glyph_pixels("Kb7mQ", Deformations::none());
            let (l, t, r, b) = bbox(&again);
            assert!(
                l.abs_diff(l0) <= 1
                    && t.abs_diff(t0) <= 1
                    && r.abs_diff(r0) <= 1
                    && b.abs_diff(b0) <= 1,
                "placement moved with every intensity at zero: \
                 ({l},{t},{r},{b}) vs ({l0},{t0},{r0},{b0})"
            );
        }
    }

    #[test]
    fn test_jitter_moves_letters_and_respects_its_bound() {
        let base = glyph_pixels("Kb7mQ", Deformations::none());
        let (bl, bt, br, bb) = bbox(&base);

        // 5 characters render at SCALE_MD, so this is the largest shift a
        // full-intensity jitter can produce, plus one pixel for rounding.
        let limit = (MAX_JITTER * SCALE_MD).round() as u32 + 1;

        let deform = Deformations {
            jitter: 1.0,
            ..Deformations::none()
        };

        let mut moved = false;
        for _ in 0..24 {
            let jittered = glyph_pixels("Kb7mQ", deform);
            if jittered != base {
                moved = true;
            }
            let (l, t, r, b) = bbox(&jittered);
            assert!(
                l + limit >= bl && t + limit >= bt && r <= br + limit && b <= bb + limit,
                "jittered bbox ({l},{t},{r},{b}) escaped ({bl},{bt},{br},{bb}) by more than {limit}px"
            );
        }
        assert!(
            moved,
            "full-intensity jitter should move at least one letter"
        );
    }

    #[test]
    fn test_scale_variance_resizes_letters_while_pinning_the_baseline() {
        // No descenders, so the bottom of the ink *is* the baseline.
        let base = glyph_pixels("KBMX", Deformations::none());
        let (_, base_top, _, base_bottom) = bbox(&base);

        let deform = Deformations {
            scale: 1.0,
            ..Deformations::none()
        };

        let mut tops = BTreeSet::new();
        for _ in 0..24 {
            let scaled = glyph_pixels("KBMX", deform);
            let (_, top, _, bottom) = bbox(&scaled);
            tops.insert(top);
            assert!(
                bottom.abs_diff(base_bottom) <= 3,
                "baseline drifted: bottom {bottom} vs {base_bottom}"
            );
        }

        // Full intensity varies cap height by up to 30% of roughly 30px, so a
        // real spread is several pixels — comfortably above the one-pixel
        // wobble the colour-dependent edges can produce on their own.
        let spread_px = tops.last().unwrap() - tops.first().unwrap();
        assert!(
            spread_px >= 3,
            "scale variance barely changed letter height ({spread_px}px); tops {tops:?}"
        );
        assert!(
            tops.iter().any(|t| t.abs_diff(base_top) >= 2),
            "no render differed meaningfully in height from the undeformed one"
        );
    }

    #[test]
    fn test_skew_leans_letters_without_changing_their_height() {
        let base = glyph_pixels("KBMX", Deformations::none());
        let (base_left, base_top, base_right, base_bottom) = bbox(&base);
        let base_width = base_right - base_left;

        let deform = Deformations {
            skew: 1.0,
            ..Deformations::none()
        };

        let mut widened = 0;
        for _ in 0..24 {
            let leaned = glyph_pixels("KBMX", deform);
            let (l, t, r, b) = bbox(&leaned);
            if r - l > base_width {
                widened += 1;
            }
            // Shearing about the letter's middle tilts it without moving it up
            // or down, so the vertical extent should be untouched.
            assert!(
                t.abs_diff(base_top) <= 2 && b.abs_diff(base_bottom) <= 2,
                "skew changed the vertical extent: ({t},{b}) vs ({base_top},{base_bottom})"
            );
        }
        assert!(
            widened >= 20,
            "a leaning letter is wider than an upright one; only {widened}/24 were"
        );
    }

    #[test]
    fn test_difficulty_one_deforms_nothing_and_ten_is_full_intensity() {
        assert_eq!(
            Deformations::for_difficulty(1),
            Deformations::none(),
            "the easiest level must render as it always did"
        );

        let hardest = Deformations::for_difficulty(10);
        for value in [hardest.jitter, hardest.scale, hardest.skew, hardest.wave] {
            assert!(
                (value - 1.0).abs() < 1e-6,
                "level 10 should be full: {value}"
            );
        }

        // Monotonic in between, and every deformation moves together.
        let mut previous = Deformations::none();
        for level in 2..=10 {
            let current = Deformations::for_difficulty(level);
            assert!(
                current.jitter > previous.jitter
                    && current.scale > previous.scale
                    && current.skew > previous.skew
                    && current.wave > previous.wave,
                "intensity should rise at every level; stalled at {level}"
            );
            previous = current;
        }

        assert_eq!(Deformations::for_difficulty(0), Deformations::none());
        assert_eq!(Deformations::for_difficulty(999), hardest);
    }

    /// Compares medians rather than counting how many individual draws come
    /// out wider, because a per-draw count is the wrong statistic here: jitter
    /// is symmetric and scale can shrink a letter, so roughly 15% of
    /// full-intensity draws are actually *narrower* than the undeformed
    /// baseline. A threshold on that count sits on top of its own mean and
    /// flakes; the median over the same draws is stable, and the gap it has to
    /// clear (median 184 against a baseline of 176, with the lower quartile at
    /// 180) leaves real margin.
    #[test]
    fn test_a_harder_captcha_disturbs_its_letters_more() {
        let easy = glyph_pixels("KBMX", Deformations::for_difficulty(1));
        let (easy_left, _, easy_right, _) = bbox(&easy);
        let easy_width = easy_right - easy_left;

        let mut widths: Vec<u32> = (0..24)
            .map(|_| {
                let hard = glyph_pixels("KBMX", Deformations::for_difficulty(10));
                let (l, _, r, _) = bbox(&hard);
                r - l
            })
            .collect();
        widths.sort_unstable();
        let median = widths[widths.len() / 2];

        assert!(
            median > easy_width,
            "difficulty 10 should spread letters wider than difficulty 1: \
             median {median} vs {easy_width} (widths {widths:?})"
        );
    }

    #[test]
    fn test_font_parses_and_is_cached() {
        let a = font();
        let b = font();
        assert!(std::ptr::eq(a, b), "font should be parsed once");
    }

    /// The bundled font is subset to exactly BASIC_CHAR. If a character is
    /// added to the set without rerunning scripts/subset-font.py it maps to
    /// .notdef (glyph 0) and renders as a blank or a box.
    #[test]
    fn test_every_basic_char_has_a_glyph() {
        let font = font();
        let missing: Vec<char> = BASIC_CHAR
            .iter()
            .copied()
            .filter(|c| font.glyph_id(*c).0 == 0)
            .collect();
        assert!(
            missing.is_empty(),
            "characters missing from the font subset: {missing:?} — \
             rerun scripts/subset-font.py"
        );

        // Prove the check above can actually fail: '1' and 'O' are excluded
        // from BASIC_CHAR as look-alikes, so they are absent from the subset
        // and must resolve to .notdef.
        for excluded in ['1', 'O', '@'] {
            assert!(!BASIC_CHAR.contains(&excluded));
            assert_eq!(
                font.glyph_id(excluded).0,
                0,
                "{excluded:?} should not be in the subset; \
                 if it is, this guard cannot detect a stale font"
            );
        }
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
