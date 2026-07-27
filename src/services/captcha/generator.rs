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
    composite_mask, composite_mask_gradient, draw_cubic_bezier_curve_mut, draw_hollow_circle_mut,
    gaussian_noise_mut, rasterize_char, salt_and_pepper_noise_mut, GlyphMask, Hsl,
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

/// Glyph lightness, chosen to stay legible against each background.
///
/// The light background sits near 93% lightness and the dark one near 7%, so
/// these ranges keep every glyph well clear of its ground whatever hue it draws.
const LIGHT_MODE_LIGHTNESS: std::ops::Range<f32> = 0.32..0.52;
const DARK_MODE_LIGHTNESS: std::ops::Range<f32> = 0.55..0.80;

/// Glyph saturation. The floor keeps colours from washing out toward grey,
/// where they would blend into the noise rather than stand against it.
const GLYPH_SATURATION: std::ops::Range<f32> = 0.50..0.95;

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

/// At full intensity, a letter turns by up to this many radians about its own
/// centre — 0.45 being roughly 26 degrees either way.
///
/// Rotation and skew both tilt a letter, but they are not the same deformation
/// and neither subsumes the other. A shear leaves horizontals horizontal, so
/// the crossbar of an `A` and the baseline of an `L` stay level and a solver
/// keeps a reliable horizon; a rotation takes those with it. That is the point
/// of having it: without rotation every glyph sits on one shared line, and the
/// line itself is a free segmentation cue — find it, and you know where the
/// letters are even when you cannot yet read them.
///
/// The bound is legibility. Past about 30 degrees the reversible pairs start
/// trading places — a rotated `N` reads as `Z`, `M` as `W`, `6` as `9` — which
/// costs a human the character outright while costing a solver that has the
/// character set nothing it cannot brute-force.
const MAX_ROTATION: f32 = 0.45;

/// At full intensity, the sine wave pushes a row sideways by up to this
/// fraction of the font size.
const MAX_WAVE_AMPLITUDE: f32 = 0.14;

/// The wave's period, as a multiple of the letter's height. Under 1.0 a letter
/// shows more than a full cycle, which reads as a wobble rather than a bend.
const WAVE_PERIOD: std::ops::Range<f32> = 0.7..1.6;

/// At full intensity, the letters are laid out across this fraction of the
/// width they would otherwise occupy.
///
/// Letters keep their size and only the gaps between them close, so glyphs
/// crowd into each other and overlap. At 0.45 with five characters the step
/// drops from roughly 42px to 19px against glyphs 25-30px wide, so neighbours
/// genuinely intersect rather than merely sitting close.
const MAX_CLUSTERING: f32 = 0.45;

/// At full intensity, this fraction of a solution's letters are drawn hollow.
///
/// A share rather than all of them, because a mixture costs a solver more than a
/// rule does. If every letter were outlined, "hollow" would simply be the
/// font — one more fixed property to template-match against, and the renderer is
/// open source, so it would be a known one. With some letters filled and some
/// not, neither a stroke detector nor a filled-blob detector describes the whole
/// solution, and which letters are which changes per image.
const MAX_OUTLINE_SHARE: f32 = 0.6;

/// Outline stroke width, as a fraction of the font size.
///
/// Randomised per letter for the same reason the colours are: a fixed stroke
/// width is an enumerable constant. The range is bounded below by legibility —
/// Roboto Bold's stems are around 0.14 of the font size, so a stroke much over
/// 0.055 closes the counter back up and the letter merely looks filled and
/// slightly thin, while one much under 0.025 disappears into JPEG artifacts at
/// the compression this service ships.
const OUTLINE_STROKE: std::ops::Range<f32> = 0.025..0.055;

/// At full intensity, a letter's opacity ramps from solid down to as low as this
/// across the letter.
///
/// The floor is set by what survives difficulty-10 noise: a glyph at 0.35 opacity
/// on the light background still lands around 25% of the way from ground to full
/// ink, which is above the gaussian noise at that level but not comfortably so.
/// Only the far end of the ramp reaches it — see `Shading::draw`, where the near
/// end is pinned at full opacity so every letter keeps an anchor.
const MIN_OPACITY: f32 = 0.35;

/// The opacity floor for a letter that is *also* outlined.
///
/// An outlined letter has an order of magnitude less ink than a filled one, and
/// all of it near the edge where the fade and the antialiasing compound. Fading
/// a hairline to 0.35 leaves nothing a human can follow, so the two deformations
/// are allowed to combine only at this shallower depth.
const MIN_OUTLINE_OPACITY: f32 = 0.6;

/// At full intensity, the far end of a letter's gradient sits up to this far
/// from the near end on the hue wheel, in turns — 0.5 being the opposite side.
const MAX_GRADIENT_HUE_SHIFT: f32 = 0.45;

/// At full intensity, a letter is defocused with a gaussian of up to this sigma,
/// as a fraction of the font size.
///
/// Drawn per letter from zero up to the cap, so a solution mixes sharp and soft
/// glyphs rather than being uniformly out of focus — the same argument as
/// `MAX_OUTLINE_SHARE`. At 42px the cap is a sigma of about 1.9px, which visibly
/// softens a stroke without dissolving it.
const MAX_BLUR: f32 = 0.045;

/// The blur cap for a letter that is *also* outlined.
///
/// An outline stroke is 1-2.75px wide, and a gaussian whose sigma approaches the
/// stroke width does not soften the stroke so much as erase it — the two edges
/// blur into each other and the counter fills back in, undoing the hollowing.
/// This keeps sigma well inside the thinnest stroke `OUTLINE_STROKE` can draw.
const MAX_OUTLINE_BLUR: f32 = 0.015;

/// The intensity `blur` is held at for every difficulty above the easiest.
///
/// **Blur deliberately does not ramp with difficulty, and it is the only
/// deformation that does not.** It used to, and that was measured to do
/// nothing: 3 grids of solver runs — an unpaired one at difficulty 3/5/8/10 and
/// a paired crossover at 6 and 7 — all came back inside noise. The diagnosis
/// was that the ramp put the deformation in the wrong place. Sigma reached
/// about 0.35px at difficulty 3, which is invisible, and full strength only at
/// 8 and 10 where every arm already scores zero and there is no solve rate left
/// to take away. So it was absent where it could have helped and saturated
/// where nothing can.
///
/// Holding it flat puts full blur at difficulty 3 and 5, which is the only band
/// where the measurement has the headroom to detect an effect at all. If it
/// does not move those, the deformation costs 5-8% of render time for nothing
/// and should be deleted rather than retuned again.
const FLAT_BLUR: f32 = 1.0;

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
    /// Random per-letter turn about its own centre, so the solution does not
    /// sit on one shared baseline.
    pub rotation: f32,
    /// How tightly the letters are packed. Higher values shrink the span they
    /// are laid out across, without shrinking the letters, so they overlap.
    pub clustering: f32,
    /// Chance that a given letter is drawn hollow — an outline with no fill.
    pub outline: f32,
    /// How far a letter's opacity may drop, and how steeply it may ramp across
    /// the letter.
    pub transparency: f32,
    /// How far the two ends of a letter's colour gradient may diverge. At zero a
    /// letter is painted in one flat colour, as it always was.
    pub gradient: f32,
    /// How strongly a letter may be defocused.
    pub blur: f32,
}

impl Deformations {
    /// No deformation — the pre-existing rendering.
    pub const fn none() -> Self {
        Self {
            jitter: 0.0,
            scale: 0.0,
            skew: 0.0,
            wave: 0.0,
            rotation: 0.0,
            clustering: 0.0,
            outline: 0.0,
            transparency: 0.0,
            gradient: 0.0,
            blur: 0.0,
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
    /// `MAX_SKEW`, `MAX_WAVE_AMPLITUDE`, `MAX_ROTATION`, `MAX_OUTLINE_SHARE`,
    /// `MIN_OPACITY`, `MAX_GRADIENT_HUE_SHIFT`, `MAX_BLUR` — so retuning how strong
    /// one gets at a given level is a change to that constant, not to this ramp.
    ///
    /// `blur` is the one exception to the ramp and is pinned at [`FLAT_BLUR`];
    /// the reasoning is on that constant. Difficulty 1 still means *no*
    /// deformation, blur included, because that is the contract the untouched
    /// output tests rest on.
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
            rotation: intensity,
            clustering: intensity,
            outline: intensity,
            transparency: intensity,
            gradient: intensity,
            blur: FLAT_BLUR,
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
///
/// The hue is unconstrained; only lightness and saturation are bounded, and
/// only enough to keep the glyph legible against its background.
///
/// This replaced a fixed palette of five colours per mode, which was a
/// segmentation key. An attacker who read the source — the renderer is open —
/// could separate glyph pixels from noise by testing membership in five known
/// RGB values, which is exactly how one solver approached these images before
/// template-matching against the bundled font. A continuous hue leaves nothing
/// to enumerate.
fn get_color(dark_mode: bool) -> Rgb<u8> {
    get_hsl(dark_mode).to_rgb()
}

/// The lightness band that keeps a glyph legible against the given background.
fn glyph_lightness(dark_mode: bool) -> std::ops::Range<f32> {
    if dark_mode {
        DARK_MODE_LIGHTNESS
    } else {
        LIGHT_MODE_LIGHTNESS
    }
}

/// [`get_color`] before conversion, so a caller can derive a second, related
/// colour from it for a gradient and interpolate in the space both were drawn in.
fn get_hsl(dark_mode: bool) -> Hsl {
    let mut rng = rng();
    Hsl {
        hue: rng.random_range(0.0..1.0),
        saturation: rng.random_range(GLYPH_SATURATION),
        lightness: rng.random_range(glyph_lightness(dark_mode)),
    }
}

/// How one letter is painted, as distinct from where it lands.
///
/// Drawn per letter and independently of the geometry, so a hollow letter can
/// also be faded, a faded one can also carry a gradient, and none of it
/// correlates with position. Every field's random draw is skipped outright when
/// its intensity is zero — the same discipline as [`spread`], for the same
/// reason: switching one deformation off must not shift another's draws.
struct Shading {
    /// Colour at the near end of the gradient, and the whole letter's colour
    /// when there is no gradient.
    near: Hsl,
    /// Outline stroke in pixels, or 0.0 for a solid letter.
    stroke: f32,
    /// Opacity at the far end of the fade, and the fade's axis in radians. The
    /// near end is always fully opaque, for the reason given in [`Shading::draw`].
    fade: Option<(f32, f32)>,
    /// Far end of the colour gradient, and its axis in radians.
    gradient: Option<(Hsl, f32)>,
    /// Gaussian sigma in pixels, or 0.0 for a sharp letter.
    sigma: f32,
}

impl Shading {
    /// `scale` is the nominal font size, which the stroke width is taken as a
    /// fraction of.
    fn draw(dark_mode: bool, scale: f32, deform: Deformations) -> Self {
        let near = get_hsl(dark_mode);

        let stroke = if deform.outline > 0.0
            && rng().random_range(0.0..1.0) < deform.outline.clamp(0.0, 1.0) * MAX_OUTLINE_SHARE
        {
            scale * rng().random_range(OUTLINE_STROKE)
        } else {
            0.0
        };

        let fade = (deform.transparency > 0.0).then(|| {
            let floor = if stroke > 0.0 {
                MIN_OUTLINE_OPACITY
            } else {
                MIN_OPACITY
            };
            // Interpolating the floor rather than the opacity itself keeps a low
            // intensity genuinely mild: at 0.1 nothing drops below 0.94, whatever
            // the far end happens to draw.
            let floor = 1.0 - (1.0 - floor) * deform.transparency.clamp(0.0, 1.0);
            let mut rng = rng();
            // The near end stays fully opaque and only the far end fades, so the
            // fade is always a ramp across the letter and never a uniform wash.
            // That is deliberate, and it was measured: drawing both ends freely
            // let a letter come out evenly faint, which is a loss of contrast
            // over the whole glyph — it costs a human the letter outright while
            // costing a machine nothing, since a global contrast change is one
            // normalisation away from undone. A ramp instead defeats any single
            // threshold *within* a letter while leaving an anchor at full ink for
            // a human to follow it from. The direction is random, so which part
            // of the letter is solid is not predictable.
            (
                rng.random_range(floor..1.0),
                rng.random_range(0.0..std::f32::consts::TAU),
            )
        });

        // Capped harder when the letter is hollow, for the reason on
        // MAX_OUTLINE_BLUR: a sigma near the stroke width closes the counter
        // back up and undoes the outline.
        let sigma = if deform.blur > 0.0 {
            let cap = if stroke > 0.0 {
                MAX_OUTLINE_BLUR
            } else {
                MAX_BLUR
            };
            scale * cap * deform.blur.clamp(0.0, 1.0) * rng().random_range(0.0..1.0)
        } else {
            0.0
        };

        let gradient = (deform.gradient > 0.0).then(|| {
            let intensity = deform.gradient.clamp(0.0, 1.0);
            let mut rng = rng();
            let hue = near.hue
                + rng.random_range(-MAX_GRADIENT_HUE_SHIFT..MAX_GRADIENT_HUE_SHIFT) * intensity;
            // Lightness ramps as well as hue, and that is the half that costs a
            // solver something: a hue shift alone leaves a letter uniformly dark
            // against a light ground, so any brightness threshold still finds
            // all of it. Drawing the far end from the same legible band and then
            // interpolating towards it by the intensity keeps both ends readable
            // at every level.
            let far = rng.random_range(glyph_lightness(dark_mode));
            (
                Hsl {
                    hue,
                    saturation: near.saturation,
                    lightness: near.lightness + (far - near.lightness) * intensity,
                },
                rng.random_range(0.0..std::f32::consts::TAU),
            )
        });

        Self {
            near,
            stroke,
            fade,
            gradient,
            sigma,
        }
    }

    /// Applies the deformations that change the mask itself.
    fn shape(&self, mask: GlyphMask) -> GlyphMask {
        // Outlined before faded, so the fade attenuates the stroke rather than
        // the ink the stroke is about to be cut from.
        let mask = if self.stroke > 0.0 {
            mask.outline(self.stroke)
        } else {
            mask
        };
        // Defocus after hollowing and before fading: blurring first would give
        // the erosion a soft mask to cut from and produce a wide smear rather
        // than a soft stroke, and fading first would be undone here anyway,
        // since the blur redistributes the very coverage the fade just scaled.
        let mask = if self.sigma > 0.0 {
            mask.blur(self.sigma)
        } else {
            mask
        };
        match self.fade {
            Some((far, angle)) => mask.fade(1.0, far, angle),
            None => mask,
        }
    }

    /// Blends the finished mask onto the canvas.
    fn paint(&self, image: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, mask: &GlyphMask, x: i32, y: i32) {
        match self.gradient {
            Some((far, angle)) => composite_mask_gradient(image, mask, x, y, self.near, far, angle),
            None => composite_mask(image, mask, x, y, self.near.to_rgb()),
        }
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

    // Clustering shrinks the span the letters are laid out across and recentres
    // it, so glyphs crowd toward the middle and overlap. The letters themselves
    // are untouched — only the gaps close — which is what turns segmentation
    // into a guess rather than just making each glyph harder to recognise.
    //
    // At zero this reproduces the original integer stepping exactly, rather
    // than merely closely: `usable_width / n` truncates, and a float span would
    // place letters a pixel or two off for lengths that do not divide evenly.
    let (origin, step) = if deform.clustering > 0.0 {
        let full = usable_width as f32;
        let span = full * (1.0 - (1.0 - MAX_CLUSTERING) * deform.clustering.clamp(0.0, 1.0));
        (5.0 + (full - span) / 2.0, span / chars.len() as f32)
    } else {
        (5.0, (usable_width / chars.len() as u32) as f32)
    };

    let y = (image.height() / 2).saturating_sub(15) as i32;

    let scale = match chars.len() {
        1..=3 => SCALE_LG,
        4..=5 => SCALE_MD,
        _ => SCALE_SM,
    };

    let font = font();
    let nominal_ascent = font.as_scaled(PxScale::from(scale)).ascent();

    for (i, ch) in chars.iter().enumerate() {
        let x = (origin + i as f32 * step).round() as i32;
        // Drawn unconditionally so the draw count does not depend on whether
        // this glyph happens to have an outline in the font.
        let shading = Shading::draw(dark_mode, scale, deform);

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

        // Drawn outside the `if let` for the same reason as the shading: the
        // number of random draws must not depend on whether this particular
        // glyph turned out to have an outline in the font.
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
        let turn = spread(MAX_ROTATION * deform.rotation);

        if let Some(mask) = rasterize_char(font, *ch, px) {
            let mask = if lean != 0.0 || amplitude != 0.0 || turn != 0.0 {
                // The shear and the wave are both horizontal displacements that
                // depend only on the row, so they sum into one closure. The
                // rotation cannot join that sum — it moves ink vertically too —
                // but it shares the same resample, which is what matters:
                // filtering the glyph twice would soften it for no reason.
                //
                // The shear is taken about the letter's middle so it leans in
                // place, and the wave's period scales with the letter's height
                // so a tall glyph is not cut into more cycles than a short one.
                let centre = mask.height as f32 / 2.0;
                let wavelength = (mask.height as f32 * period).max(1.0);
                mask.displace_and_rotate(
                    |row| {
                        lean * (row - centre)
                            + amplitude * (std::f32::consts::TAU * row / wavelength + phase).sin()
                    },
                    turn,
                )
            } else {
                mask
            };
            // Hollowing and fading come after the displacement, not before: a
            // stroke a pixel or two wide, resampled by the shear, would soften
            // into a smear, and the point of an outline is a crisp edge.
            let mask = shading.shape(mask);
            shading.paint(image, &mask, x + dx, y + dy - baseline_shift);
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

        let cluster: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        clustering: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&cluster, 3), "06-clustering.jpg");

        let outline: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        outline: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&outline, 3), "07-outline.jpg");

        let transparency: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        transparency: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&transparency, 3), "08-transparency.jpg");

        let gradient: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        gradient: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&gradient, 3), "09-gradient.jpg");

        // The three new deformations together, since each acts on the same ink
        // and it is their combination that has to stay legible.
        let painted: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        outline: *i,
                        transparency: *i,
                        gradient: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&painted, 3), "10-outline-fade-gradient.jpg");

        let blur: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        blur: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&blur, 3), "11-blur.jpg");

        // Blur off against blur on at the difficulties a caller asks for. This
        // is the sheet the keep-or-drop decision rests on. It used to show
        // nothing at difficulty 3 — the ramp made sigma too small to see there,
        // and by the time it was strong the letters were unreadable for other
        // reasons. `FLAT_BLUR` is the answer to that, so the low-difficulty rows
        // are now where the difference is actually visible.
        let mut comparison = Vec::new();
        for difficulty in [3u32, 5, 8, 10] {
            let with = Deformations::for_difficulty(difficulty);
            comparison.push((difficulty, Deformations { blur: 0.0, ..with }));
            comparison.push((difficulty, with));
        }
        write_jpeg(&contact_sheet(&comparison, 2), "12-blur-off-vs-on.jpg");

        // Everything on, at the difficulty levels a caller actually asks for,
        // so the noise and the deformations ramp together.
        let by_difficulty: Vec<_> = [1u32, 3, 5, 7, 10]
            .iter()
            .map(|d| (*d, Deformations::for_difficulty(*d)))
            .collect();
        write_jpeg(&contact_sheet(&by_difficulty, 3), "13-by-difficulty.jpg");

        let rotation: Vec<_> = levels
            .iter()
            .map(|i| {
                (
                    1,
                    Deformations {
                        rotation: *i,
                        ..Deformations::none()
                    },
                )
            })
            .collect();
        write_jpeg(&contact_sheet(&rotation, 3), "14-rotation.jpg");

        // Rotation against the skew it is most easily confused with, at matched
        // intensity. The distinction the sheet should make visible: under a
        // shear the crossbars and feet stay level, under a rotation they do not.
        let mut tilt = Vec::new();
        for intensity in [0.5f32, 1.0] {
            tilt.push((
                1,
                Deformations {
                    skew: intensity,
                    ..Deformations::none()
                },
            ));
            tilt.push((
                1,
                Deformations {
                    rotation: intensity,
                    ..Deformations::none()
                },
            ));
        }
        write_jpeg(&contact_sheet(&tilt, 3), "15-skew-vs-rotation.jpg");

        // Rotation off against rotation on at shipping difficulties, the same
        // shape as the blur comparison and for the same decision.
        let mut turned = Vec::new();
        for difficulty in [3u32, 5, 8, 10] {
            let with = Deformations::for_difficulty(difficulty);
            turned.push((
                difficulty,
                Deformations {
                    rotation: 0.0,
                    ..with
                },
            ));
            turned.push((difficulty, with));
        }
        write_jpeg(&contact_sheet(&turned, 2), "16-rotation-off-vs-on.jpg");
    }

    /// Switches one deformation off, leaving the rest of the level untouched.
    ///
    /// The A/B harnesses below both need "this level, minus one deformation",
    /// and doing it by name keeps them generic — a new deformation becomes
    /// measurable by adding a line here rather than by copying a test.
    fn without(deform: Deformations, field: &str) -> Deformations {
        match field {
            "jitter" => Deformations {
                jitter: 0.0,
                ..deform
            },
            "scale" => Deformations {
                scale: 0.0,
                ..deform
            },
            "skew" => Deformations {
                skew: 0.0,
                ..deform
            },
            "wave" => Deformations {
                wave: 0.0,
                ..deform
            },
            "rotation" => Deformations {
                rotation: 0.0,
                ..deform
            },
            "clustering" => Deformations {
                clustering: 0.0,
                ..deform
            },
            "outline" => Deformations {
                outline: 0.0,
                ..deform
            },
            "transparency" => Deformations {
                transparency: 0.0,
                ..deform
            },
            "gradient" => Deformations {
                gradient: 0.0,
                ..deform
            },
            "blur" => Deformations {
                blur: 0.0,
                ..deform
            },
            other => panic!("unknown deformation {other:?}"),
        }
    }

    /// Only the named deformation, at `intensity`, with nothing else on.
    ///
    /// By name rather than by setter closure — [`only`] covers that case — so
    /// the measurement harnesses can be pointed at a deformation from the
    /// command line.
    fn only_named(field: &str, intensity: f32) -> Deformations {
        let mut all = Deformations::none();
        match field {
            "jitter" => all.jitter = intensity,
            "scale" => all.scale = intensity,
            "skew" => all.skew = intensity,
            "wave" => all.wave = intensity,
            "rotation" => all.rotation = intensity,
            "clustering" => all.clustering = intensity,
            "outline" => all.outline = intensity,
            "transparency" => all.transparency = intensity,
            "gradient" => all.gradient = intensity,
            "blur" => all.blur = intensity,
            other => panic!("unknown deformation {other:?}"),
        }
        all
    }

    /// Isolates what one deformation costs: render time and encoded size, off
    /// against on with everything else held identical.
    ///
    /// Ignored because it measures rather than asserts — a timing threshold in
    /// CI would be a flake generator:
    ///
    /// ```text
    /// CAPTCHA_AB_FIELD=rotation \
    ///   cargo test --release --lib deformation_impact -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measures render time and encoded size, asserts nothing"]
    fn deformation_impact() {
        use image::codecs::jpeg::JpegEncoder;
        use std::time::Instant;

        const DRAWS: usize = 400;
        const QUALITY: u8 = 40;

        let field = std::env::var("CAPTCHA_AB_FIELD").unwrap_or_else(|_| "blur".to_string());

        fn measure(difficulty: u32, deform: Deformations) -> (u128, usize) {
            let mut times = Vec::with_capacity(DRAWS);
            let mut sizes = Vec::with_capacity(DRAWS);
            for _ in 0..DRAWS {
                let start = Instant::now();
                let image = render(SAMPLE_TEXT, difficulty, SAMPLE_W, SAMPLE_H, false, deform);
                times.push(start.elapsed().as_micros());
                let mut bytes = Vec::new();
                JpegEncoder::new_with_quality(&mut bytes, QUALITY)
                    .encode_image(&image)
                    .expect("encodes");
                sizes.push(bytes.len());
            }
            times.sort_unstable();
            sizes.sort_unstable();
            (times[DRAWS / 2], sizes[DRAWS / 2])
        }

        // Warm the lazily-parsed font so its one-time cost lands on neither arm.
        let _ = render(
            SAMPLE_TEXT,
            5,
            SAMPLE_W,
            SAMPLE_H,
            false,
            Deformations::none(),
        );

        println!("\n{field} impact, medians over {DRAWS} draws at quality {QUALITY}\n");
        println!(
            "{:>4} {:>9} {:>9} {:>8}   {:>9} {:>9} {:>8}",
            "diff", "us off", "us on", "delta", "bytes off", "bytes on", "delta"
        );
        for difficulty in [1u32, 3, 5, 8, 10] {
            let with = Deformations::for_difficulty(difficulty);
            let (t_off, s_off) = measure(difficulty, without(with, &field));
            let (t_on, s_on) = measure(difficulty, with);
            let pct = |a: f64, b: f64| 100.0 * (b - a) / a;
            println!(
                "{difficulty:>4} {t_off:>9} {t_on:>9} {:>7.1}%   {s_off:>9} {s_on:>9} {:>7.1}%",
                pct(t_off as f64, t_on as f64),
                pct(s_off as f64, s_on as f64)
            );
        }

        // The deformation alone, no others and no noise, so the primitive's own
        // cost is visible rather than buried under the gaussian noise pass.
        let (bare_t, bare_s) = measure(1, Deformations::none());
        let (solo_t, solo_s) = measure(1, only_named(&field, 1.0));
        println!(
            "\n{field} alone at difficulty 1: {bare_t}us -> {solo_t}us ({:+.1}%), \
             {bare_s}B -> {solo_s}B ({:+.1}%)",
            100.0 * (solo_t as f64 - bare_t as f64) / bare_t as f64,
            100.0 * (solo_s as f64 - bare_s as f64) / bare_s as f64
        );
    }

    /// Writes a paired A/B image set for solver evaluation.
    ///
    /// Two design points, both learned from getting them wrong. It targets the
    /// difficulty band where there is headroom to detect anything at all —
    /// below it every arm solves everything and above it every arm solves
    /// nothing, so neither end can move. And it renders the *same solution
    /// text* under both conditions, so per-string difficulty cancels instead of
    /// adding variance; an unpaired comparison is partly measuring whether
    /// `VeDY` is harder than `P9tD`.
    ///
    /// Emits both versions of every text. The caller must split them so no
    /// solver sees a string twice — otherwise the second sighting is a memory
    /// test, not a vision test. `pair` in the manifest is what to split on.
    ///
    /// ```text
    /// CAPTCHA_SAMPLE_DIR=/tmp/rotation-ab CAPTCHA_AB_FIELD=rotation \
    ///   CAPTCHA_AB_LEVELS=3,5 \
    ///   cargo test --release --lib deformation_ab_set -- --ignored
    /// ```
    #[test]
    #[ignore = "writes a paired A/B image set, asserts nothing"]
    fn deformation_ab_set() {
        use image::codecs::jpeg::JpegEncoder;

        let dir = std::env::var("CAPTCHA_SAMPLE_DIR").unwrap_or_else(|_| "/tmp".to_string());
        std::fs::create_dir_all(&dir).expect("output directory is writable");
        let field = std::env::var("CAPTCHA_AB_FIELD").unwrap_or_else(|_| "blur".to_string());
        let levels: Vec<u32> = std::env::var("CAPTCHA_AB_LEVELS")
            .unwrap_or_else(|_| "6,7".to_string())
            .split(',')
            .map(|level| {
                level
                    .trim()
                    .parse()
                    .expect("difficulty levels are integers")
            })
            .collect();

        let mut manifest = Vec::new();
        let mut index = 0;
        let mut pair = 0;

        for difficulty in levels {
            for length in [4usize, 5, 6] {
                for _ in 0..3 {
                    // One text, both conditions — the whole point of the pairing.
                    let text = random_text(length);
                    let with = Deformations::for_difficulty(difficulty);

                    for (condition, deform) in [("off", without(with, &field)), ("on", with)] {
                        let image = render(&text, difficulty, SAMPLE_W, SAMPLE_H, false, deform);
                        let mut bytes = Vec::new();
                        JpegEncoder::new_with_quality(&mut bytes, 40)
                            .encode_image(&image)
                            .expect("encodes");
                        let name = format!("{index:03}.jpg");
                        std::fs::write(format!("{dir}/{name}"), &bytes).expect("image writes");
                        manifest.push(format!(
                            "  {{\"file\": \"{name}\", \"solution\": \"{text}\", \
                             \"length\": {length}, \"difficulty\": {difficulty}, \
                             \"field\": \"{field}\", \"condition\": \"{condition}\", \
                             \"pair\": {pair}, \"bytes\": {}}}",
                            bytes.len()
                        ));
                        index += 1;
                    }
                    pair += 1;
                }
            }
        }

        std::fs::write(
            format!("{dir}/manifest.json"),
            format!("[\n{}\n]\n", manifest.join(",\n")),
        )
        .expect("manifest writes");
        println!("wrote {index} images ({pair} texts x 2 conditions) to {dir}/");
    }

    /// Letters only, on a bare canvas — no interference lines, ellipses or
    /// noise — so a deformation can be observed without random clutter on top.
    fn glyph_canvas(text: &str, deform: Deformations) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
        let mut image = background(220, 120, false);
        write_characters(text, &mut image, false, deform);
        image
    }

    fn glyph_pixels(text: &str, deform: Deformations) -> BTreeSet<(u32, u32)> {
        glyph_canvas(text, deform)
            .enumerate_pixels()
            .filter(|(_, _, p)| p.0 != LIGHT)
            .map(|(x, y, _)| (x, y))
            .collect()
    }

    /// The colour of every pixel a letter touched.
    fn inked_colors(text: &str, deform: Deformations) -> Vec<[u8; 3]> {
        glyph_canvas(text, deform)
            .pixels()
            .map(|p| p.0)
            .filter(|c| *c != LIGHT)
            .collect()
    }

    /// How far a pixel travelled from the background, summed over the channels.
    fn contrast(color: [u8; 3]) -> f64 {
        color
            .iter()
            .zip(LIGHT)
            .map(|(a, b)| (i32::from(*a) - i32::from(b)).abs() as f64)
            .sum()
    }

    /// How much ink a letter laid down, and how opaquely — both measured against
    /// the strongest pixel of the same render.
    ///
    /// Dividing by that peak is what makes the figures comparable across draws
    /// at all: the glyph colour comes from a band 20 points of lightness wide,
    /// so raw contrast against the background swings by a third for reasons that
    /// have nothing to do with any deformation. Against its own peak, `area` is
    /// the letter's size counted in fully-inked pixels and `opacity` is how
    /// solid its average pixel is. A hollow letter loses area while keeping
    /// opacity; a faded one loses opacity while keeping area.
    fn ink_profile(text: &str, deform: Deformations) -> (f64, f64) {
        let colors = inked_colors(text, deform);
        assert!(
            !colors.is_empty(),
            "the letters should have drawn something"
        );
        let contrasts: Vec<f64> = colors.iter().copied().map(contrast).collect();
        let peak = contrasts.iter().copied().fold(0.0, f64::max);
        let total: f64 = contrasts.iter().sum();
        (total / peak, total / peak / contrasts.len() as f64)
    }

    fn median(mut values: Vec<f64>) -> f64 {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values[values.len() / 2]
    }

    /// How much of a letter is painted in its single commonest colour.
    ///
    /// A flat letter has one exact colour across its whole interior, so this is
    /// large; a gradient has no such colour, which is the point of it.
    fn modal_color_share(text: &str, deform: Deformations) -> f64 {
        let colors = inked_colors(text, deform);
        let mut counts = std::collections::HashMap::new();
        for color in &colors {
            *counts.entry(*color).or_insert(0usize) += 1;
        }
        *counts.values().max().unwrap() as f64 / colors.len() as f64
    }

    /// A single letter, in isolation, with one deformation dialled in.
    ///
    /// One letter rather than a word because these three deformations are drawn
    /// per letter: a five-character sample would average four other draws into
    /// every measurement and blunt the signal.
    fn only(field: fn(&mut Deformations, f32), intensity: f32) -> Deformations {
        let mut deform = Deformations::none();
        field(&mut deform, intensity);
        deform
    }

    /// Hollowing removes the interior of a letter and nothing else, so it shows
    /// up as a drop in inked *area* at unchanged opacity — measured against the
    /// letter's own peak pixel, since the glyph colour is random.
    ///
    /// The share is asserted, not just the effect: `MAX_OUTLINE_SHARE` exists to
    /// leave some letters filled, and a change that outlined all of them or none
    /// would otherwise pass. Measured over 400 draws the share came out at 0.618
    /// and 0.585 against the 0.6 it is aiming for, and 0.268 and 0.270 against
    /// the 0.3 that half intensity implies.
    #[test]
    fn test_outline_hollows_a_share_of_the_letters() {
        let (filled_area, _) = ink_profile("M", Deformations::none());

        let draws = 200;
        let mut hollow = Vec::new();
        let mut filled = 0;
        for _ in 0..draws {
            let (area, _) = ink_profile("M", only(|d, i| d.outline = i, 1.0));
            if area < filled_area * 0.95 {
                // A hollow letter still has to be a letter. The thinnest stroke
                // in the range keeps around 40% of a filled letter's ink, so
                // this only fires if the erosion has started eating the stroke
                // itself rather than the interior.
                assert!(
                    area > filled_area * 0.25,
                    "an outline should not erase the letter: \
                     {area:.1} against {filled_area:.1}"
                );
                hollow.push(area);
            } else {
                // A letter is either hollowed or left alone — never nearly
                // alone. The couple of units of slack are u8 quantisation:
                // `area` divides by the render's own peak pixel, and which
                // fractional coverages round up depends on the random colour.
                assert!(
                    (area - filled_area).abs() < 5.0,
                    "an unhollowed letter should match the undeformed one: \
                     {area:.1} against {filled_area:.1}"
                );
                filled += 1;
            }
        }

        let share = hollow.len() as f64 / draws as f64;
        assert!(
            (0.48..0.72).contains(&share),
            "about {MAX_OUTLINE_SHARE} of letters should be hollow at full \
             intensity, got {share:.3} ({filled} filled of {draws})"
        );

        let widest = hollow.iter().copied().fold(0.0, f64::max);
        assert!(
            widest < filled_area * 0.9,
            "even the thickest stroke should leave a letter visibly hollow: \
             {widest:.1} against {filled_area:.1}"
        );

        // Half the intensity, half the letters — the ramp has to be in the share
        // and not only in whether the deformation happens at all.
        let half = (0..draws)
            .filter(|_| ink_profile("M", only(|d, i| d.outline = i, 0.5)).0 < filled_area * 0.95)
            .count() as f64
            / draws as f64;
        assert!(
            half < share * 0.75,
            "half intensity should hollow far fewer letters: {half:.3} against {share:.3}"
        );
    }

    /// The fade shows up as a drop in ink measured against the letter's own
    /// strongest pixel — which works *because* the near end of the ramp is
    /// pinned at full opacity. A uniform wash would cancel out in that
    /// normalisation entirely, peak and total falling together, so a
    /// regression that dropped the anchor and faded whole letters evenly would
    /// show up here as no fade at all.
    ///
    /// Measured over 120 draws across three runs: 630.7-630.8 units of ink
    /// unfaded, 587.4-591.2 at half intensity, 537.0-542.8 at full. Normalising
    /// away the random glyph colour is what makes those figures repeatable to a
    /// fraction of a percent; the same measurement in raw contrast swings by 5%
    /// run to run on colour alone.
    #[test]
    fn test_transparency_fades_letters_in_proportion_to_its_intensity() {
        let draws = 120;
        let ink_at = |deform| median((0..draws).map(|_| ink_profile("M", deform).0).collect());

        let solid = ink_at(Deformations::none());
        let half = ink_at(only(|d, i| d.transparency = i, 0.5));
        let full = ink_at(only(|d, i| d.transparency = i, 1.0));

        assert!(
            full < solid * 0.90,
            "full transparency should visibly lighten letters: {full:.1} against {solid:.1}"
        );
        assert!(
            half < solid * 0.97 && half > full * 1.05,
            "the fade should deepen with intensity: {solid:.1} -> {half:.1} -> {full:.1}"
        );

        // The anchor is not merely an average: no draw may exceed the solid
        // letter, and the deepest fades have to actually reach down towards
        // MIN_OPACITY rather than hovering just under full.
        let areas: Vec<f64> = (0..draws)
            .map(|_| ink_profile("M", only(|d, i| d.transparency = i, 1.0)).0)
            .collect();
        let widest = areas.iter().copied().fold(0.0, f64::max);
        let faintest = areas.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(
            widest <= solid + 1.0,
            "a fade can only remove ink: {widest:.1} against {solid:.1}"
        );
        assert!(
            faintest < solid * 0.8,
            "the deepest fade should be substantial: {faintest:.1} against {solid:.1}"
        );

        // Opacity, not geometry: a faded letter occupies the same place. The
        // tolerance is for the faintest antialiased edge pixels, which fade
        // below the background's own value and drop out of the bounding box.
        let (sl, st, sr, sb) = bbox(&glyph_pixels("KBMX", Deformations::none()));
        for _ in 0..12 {
            let (l, t, r, b) = bbox(&glyph_pixels("KBMX", only(|d, i| d.transparency = i, 1.0)));
            assert!(
                l.abs_diff(sl) <= 2
                    && t.abs_diff(st) <= 2
                    && r.abs_diff(sr) <= 2
                    && b.abs_diff(sb) <= 2,
                "transparency moved the letters: ({l},{t},{r},{b}) against ({sl},{st},{sr},{sb})"
            );
        }
    }

    /// A flat letter is painted in one exact colour over its whole interior, so
    /// more than half its pixels share a single value. That single value is what
    /// makes a flat letter cheap to segment, and a gradient's job is to remove
    /// it: measured over 40 draws the commonest colour covers 54% of a flat
    /// letter and 20% of a graded one.
    #[test]
    fn test_gradient_leaves_no_single_colour_describing_a_letter() {
        let draws = 40;
        let share_at =
            |deform| median((0..draws).map(|_| modal_color_share("M", deform)).collect());

        let flat = share_at(Deformations::none());
        let graded = share_at(only(|d, i| d.gradient = i, 1.0));

        assert!(
            flat > 0.4,
            "a flat letter should be dominated by one colour, got {flat:.3}"
        );
        assert!(
            graded < flat * 0.5,
            "a gradient should leave no dominant colour: {graded:.3} against {flat:.3}"
        );

        // Colour only — the letter must not move, resize or fade.
        let (fl, ft, fr, fb) = bbox(&glyph_pixels("KBMX", Deformations::none()));
        for _ in 0..12 {
            let (l, t, r, b) = bbox(&glyph_pixels("KBMX", only(|d, i| d.gradient = i, 1.0)));
            assert!(
                l.abs_diff(fl) <= 1
                    && t.abs_diff(ft) <= 1
                    && r.abs_diff(fr) <= 1
                    && b.abs_diff(fb) <= 1,
                "a gradient changed the geometry: ({l},{t},{r},{b}) against ({fl},{ft},{fr},{fb})"
            );
        }
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

    /// The whole point of rotation, stated as a measurement: without it every
    /// letter's foot sits on one shared line, and that line is a free
    /// segmentation cue. This asserts the feet scatter.
    ///
    /// Measured per letter rather than over the whole word, because the word's
    /// bounding box would also grow under a skew or a scale and would not
    /// distinguish "the letters are tilted" from "the letters are bigger".
    #[test]
    fn test_rotation_takes_letters_off_a_shared_baseline() {
        // Per-letter bottom edge, sampled in the column band each letter of a
        // four-character solution is laid out in.
        let feet = |deform: Deformations| {
            let pixels = glyph_pixels("KBMX", deform);
            let (left, _, right, _) = bbox(&pixels);
            let step = (right - left + 1) as f32 / 4.0;
            (0..4)
                .filter_map(|i| {
                    let lo = left + (i as f32 * step) as u32;
                    let hi = left + ((i + 1) as f32 * step) as u32;
                    pixels
                        .iter()
                        .filter(|(x, _)| *x >= lo && *x < hi)
                        .map(|(_, y)| *y)
                        .max()
                })
                .collect::<Vec<_>>()
        };

        // Upright, all four feet land on exactly one row — which is precisely
        // the cue worth removing, and it makes a clean zero to measure against.
        let level = feet(Deformations::none());
        let baseline = level[0];
        assert!(
            level.iter().all(|foot| *foot == baseline),
            "an undeformed solution should sit on one line, but the feet were {level:?}"
        );

        let deform = Deformations {
            rotation: 1.0,
            ..Deformations::none()
        };

        // Mean departure from that line, over every letter of every draw, not a
        // per-draw threshold: the turn is symmetric, so a single draw can come
        // out level by chance — the same flakiness the clustering test
        // documents. Averaging ~100 letters instead makes the statistic stable.
        let departures: Vec<f32> = (0..24)
            .flat_map(|_| feet(deform))
            .map(|foot| foot.abs_diff(baseline) as f32)
            .collect();
        let mean = departures.iter().sum::<f32>() / departures.len() as f32;

        assert!(
            mean > 1.0,
            "rotation should lift letters off the shared baseline; \
             mean departure was only {mean:.2}px over {} letters",
            departures.len()
        );
    }

    #[test]
    fn test_difficulty_one_deforms_nothing_and_ten_is_full_intensity() {
        assert_eq!(
            Deformations::for_difficulty(1),
            Deformations::none(),
            "the easiest level must render as it always did"
        );

        // Every field is named here, and the destructuring is what makes that
        // enforceable: add a field to `Deformations` and this stops compiling
        // until it is classified as either ramping or pinned. The previous
        // version listed accessors in a fixed-size array, which does not have
        // that property — a new field simply went unmentioned, and `blur` did
        // exactly that, uncovered by the one test that exists to catch it.
        let classify = |d: &Deformations| {
            let Deformations {
                jitter,
                scale,
                skew,
                wave,
                rotation,
                clustering,
                outline,
                transparency,
                gradient,
                blur,
            } = *d;
            // (ramping, pinned) — `blur` is deliberately flat, for the reason
            // on `FLAT_BLUR`. Nothing else joins it without a measurement.
            (
                vec![
                    ("jitter", jitter),
                    ("scale", scale),
                    ("skew", skew),
                    ("wave", wave),
                    ("rotation", rotation),
                    ("clustering", clustering),
                    ("outline", outline),
                    ("transparency", transparency),
                    ("gradient", gradient),
                ],
                vec![("blur", blur, FLAT_BLUR)],
            )
        };

        let hardest = Deformations::for_difficulty(10);
        for (name, value) in classify(&hardest).0 {
            assert!(
                (value - 1.0).abs() < 1e-6,
                "level 10 should be full for {name}: {value}"
            );
        }

        // Monotonic in between, and every ramping deformation moves together.
        let mut previous = Deformations::none();
        for level in 2..=10 {
            let current = Deformations::for_difficulty(level);
            let before = classify(&previous).0;
            for (i, (name, value)) in classify(&current).0.iter().enumerate() {
                assert!(
                    *value > before[i].1,
                    "intensity should rise at every level; {name} stalled at {level}"
                );
            }
            // The pinned ones sit at their constant from level 2 up, which is
            // the whole point: full strength at difficulty 3 and 5, where the
            // solve rate is non-zero and an effect could actually show.
            for (name, value, expected) in classify(&current).1 {
                assert!(
                    (value - expected).abs() < 1e-6,
                    "{name} should be pinned at {expected} for every level above 1, \
                     but level {level} gave {value}"
                );
            }
            previous = current;
        }

        assert_eq!(Deformations::for_difficulty(0), Deformations::none());
        assert_eq!(Deformations::for_difficulty(999), hardest);
    }

    /// Clustering inverted what this test used to assert. Before it existed,
    /// a harder CAPTCHA spread letters *wider* — jitter, scale and skew all
    /// push ink outward. Clustering pulls the whole layout in, and it
    /// dominates: measured over 300 draws, difficulty 10 spans a median of
    /// 101px against the easiest level's 176px.
    ///
    /// Medians rather than a per-draw count, for the reason that made the
    /// previous version of this test 32% flaky: jitter is symmetric, so
    /// individual draws scatter either side of the trend and a threshold on
    /// how many land the right way sits on its own mean.
    #[test]
    fn test_a_harder_captcha_packs_its_letters_tighter() {
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
            median + 30 < easy_width,
            "difficulty 10 should pack letters much tighter than difficulty 1: \
             median {median} vs {easy_width} (widths {widths:?})"
        );
    }

    /// Clustering uses no randomness at all, so the layout is deterministic
    /// given the text; only the faint antialiased edges move with the random
    /// glyph colour, which is why the margins below are a few pixels rather
    /// than exact.
    ///
    /// Measured for "Kb7mQ": 189px wide spread out, narrowing through
    /// 166 / 143 / 120 to 96 at full intensity.
    #[test]
    fn test_clustering_packs_letters_until_they_overlap() {
        let widths: Vec<u32> = [0.0f32, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|level| {
                let packed = glyph_pixels(
                    "Kb7mQ",
                    Deformations {
                        clustering: *level,
                        ..Deformations::none()
                    },
                );
                let (l, _, r, _) = bbox(&packed);
                r - l
            })
            .collect();

        for pair in widths.windows(2) {
            assert!(
                pair[1] + 2 < pair[0],
                "each step of clustering should visibly tighten the layout: {widths:?}"
            );
        }
        assert!(
            (widths[4] as f64) < widths[0] as f64 * 0.55,
            "full clustering should pull the layout under 55% of its width: {widths:?}"
        );

        // The union of inked pixels shrinking is the direct evidence of
        // overlap: the letters themselves never change size, so the only way
        // to cover fewer pixels is for glyphs to sit on top of one another.
        let spread_out = glyph_pixels("Kb7mQ", Deformations::none()).len();
        let packed = glyph_pixels(
            "Kb7mQ",
            Deformations {
                clustering: 1.0,
                ..Deformations::none()
            },
        )
        .len();
        assert!(
            (packed as f64) < spread_out as f64 * 0.97,
            "packed letters should cover fewer pixels than separated ones \
             ({packed} vs {spread_out}); if they are equal they are merely adjacent"
        );
    }

    /// Zero clustering must reproduce the original integer stepping exactly,
    /// not merely closely. `usable_width / n` truncates, so a naive float span
    /// shifts letters by a pixel or two whenever the character count does not
    /// divide the canvas evenly.
    ///
    /// These figures were measured from the commit before clustering existed
    /// and matched exactly afterwards, across every length, so they pin the
    /// old layout rather than just describing the new one.
    #[test]
    fn test_zero_clustering_preserves_the_original_layout() {
        for (text, left, width) in [
            ("KB", 7u32, 128u32),
            ("KBM", 7, 172),
            ("KBMX", 7, 176),
            ("Kb7mQ", 7, 189),
            ("KBMXQ7", 6, 189),
            ("KBMXQ7gh", 6, 196),
        ] {
            let px = glyph_pixels(text, Deformations::none());
            let (l, _, r, _) = bbox(&px);
            assert!(
                l.abs_diff(left) <= 1 && (r - l).abs_diff(width) <= 1,
                "layout drifted for {text:?}: left {l} (want {left}), width {} (want {width})",
                r - l
            );
        }
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
