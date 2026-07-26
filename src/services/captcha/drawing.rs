//! Drawing and noise primitives for the CAPTCHA renderer.
//!
//! Derived from the `imageproc` crate (v0.26.2), MIT licensed,
//! Copyright (c) 2015 PistonDevelopers — see THIRD_PARTY_LICENSES.
//!
//! Vendored to drop the dependency itself. `imageproc` declares `nalgebra`
//! unconditionally — there is no feature to switch it off — which pulled
//! `simba`, `paste` (RUSTSEC-2024-0436, unmaintained), `matrixmultiply`,
//! `num-complex`, `approx`, `safe_arch`, `typenum`, `wide` and `rawpointer`
//! into the build, plus a second copy of `rand` (0.9, against our 0.10) via
//! `rand_distr`. None of that is reachable from the six functions below.
//!
//! **Specialised to [`RgbImage`].** Upstream is generic over a `Canvas` trait
//! so callers can opt into alpha blending via its `Blend` wrapper. The
//! renderer never does: it draws straight onto an `ImageBuffer`, and the
//! blanket `impl<I: GenericImage> Canvas for I` defines `draw_pixel` as
//! `put_pixel` and `get_pixel` as `get_pixel`. Substituting the concrete
//! calls is therefore behaviour-preserving, not an approximation.
//!
//! The geometry is a line-for-line port. Only the noise generators differ, and
//! only in their source of randomness — see [`gaussian_noise_mut`].

use ab_glyph::{point, Font, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use rand::rngs::SmallRng;
use rand::{RngExt, SeedableRng};

/// `imageproc::definitions::Clamp<f32> for u8`.
///
/// Truncates rather than rounds, and the bounds are exclusive on the way in —
/// preserved exactly, since it decides every antialiased edge pixel of a glyph.
fn clamp_u8_f32(x: f32) -> u8 {
    if x < u8::MAX as f32 {
        if x > u8::MIN as f32 {
            x as u8
        } else {
            u8::MIN
        }
    } else {
        u8::MAX
    }
}

/// `imageproc::definitions::Clamp<f64> for u8`.
fn clamp_u8_f64(x: f64) -> u8 {
    if x < u8::MAX as f64 {
        if x > u8::MIN as f64 {
            x as u8
        } else {
            u8::MIN
        }
    } else {
        u8::MAX
    }
}

/// Converts HSL to RGB. `hue` is in turns (`0.0..1.0`), the rest in `0.0..=1.0`.
///
/// Hue in turns rather than degrees so a caller can draw one straight from a
/// uniform distribution without scaling, which is how every colour in the
/// renderer is now chosen.
pub fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> Rgb<u8> {
    let hue = hue.rem_euclid(1.0);
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);

    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue * 6.0;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());

    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };

    let base = lightness - chroma / 2.0;
    let to_byte = |v: f32| ((v + base) * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgb([to_byte(r), to_byte(g), to_byte(b)])
}

/// `imageproc::pixelops::weighted_sum`, specialised to `Rgb<u8>`.
fn weighted_sum(left: Rgb<u8>, right: Rgb<u8>, left_weight: f32, right_weight: f32) -> Rgb<u8> {
    let mut out = [0u8; 3];
    for (i, channel) in out.iter_mut().enumerate() {
        *channel =
            clamp_u8_f32(f32::from(left.0[i]) * left_weight + f32::from(right.0[i]) * right_weight);
    }
    Rgb(out)
}

/// Set the pixel at `(x, y)` if it lies inside the image, otherwise do nothing.
fn draw_if_in_bounds(image: &mut RgbImage, x: i32, y: i32, color: Rgb<u8>) {
    if x >= 0 && x < image.width() as i32 && y >= 0 && y < image.height() as i32 {
        image.put_pixel(x as u32, y as u32, color);
    }
}

/// Iterates over the integer points of the line between two endpoints.
///
/// Bresenham's algorithm, ported verbatim: the `as i32` truncations and the
/// `error` seeded at `dx / 2` decide which pixels land on the line, so they are
/// reproduced rather than tidied.
struct BresenhamLineIter {
    dx: f32,
    dy: f32,
    x: i32,
    y: i32,
    error: f32,
    end_x: i32,
    is_steep: bool,
    y_step: i32,
}

impl BresenhamLineIter {
    fn new(start: (f32, f32), end: (f32, f32)) -> BresenhamLineIter {
        let (mut x0, mut y0) = (start.0, start.1);
        let (mut x1, mut y1) = (end.0, end.1);

        let is_steep = (y1 - y0).abs() > (x1 - x0).abs();
        if is_steep {
            std::mem::swap(&mut x0, &mut y0);
            std::mem::swap(&mut x1, &mut y1);
        }

        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
            std::mem::swap(&mut y0, &mut y1);
        }

        let dx = x1 - x0;

        BresenhamLineIter {
            dx,
            dy: (y1 - y0).abs(),
            x: x0 as i32,
            y: y0 as i32,
            error: dx / 2f32,
            end_x: x1 as i32,
            is_steep,
            y_step: if y0 < y1 { 1 } else { -1 },
        }
    }
}

impl Iterator for BresenhamLineIter {
    type Item = (i32, i32);

    fn next(&mut self) -> Option<(i32, i32)> {
        if self.x > self.end_x {
            None
        } else {
            let ret = if self.is_steep {
                (self.y, self.x)
            } else {
                (self.x, self.y)
            };

            self.x += 1;
            self.error -= self.dy;
            if self.error < 0f32 {
                self.y += self.y_step;
                self.error += self.dx;
            }

            Some(ret)
        }
    }
}

/// Draws as much of the line segment between `start` and `end` as lies in bounds.
fn draw_line_segment_mut(image: &mut RgbImage, start: (f32, f32), end: (f32, f32), color: Rgb<u8>) {
    let (width, height) = (image.width(), image.height());
    let in_bounds = |x, y| x >= 0 && x < width as i32 && y >= 0 && y < height as i32;

    for (x, y) in BresenhamLineIter::new(start, end) {
        if in_bounds(x, y) {
            image.put_pixel(x as u32, y as u32, color);
        }
    }
}

/// Draws a hollow circle, clipped to the image bounds.
///
/// The renderer only ever asked `imageproc` for ellipses with equal radii, and
/// `draw_hollow_ellipse_mut` dispatches that case straight here — the midpoint
/// ellipse algorithm was never reachable, so it is not ported.
pub fn draw_hollow_circle_mut(
    image: &mut RgbImage,
    center: (i32, i32),
    radius: i32,
    color: Rgb<u8>,
) {
    let mut x = 0i32;
    let mut y = radius;
    let mut p = 1 - radius;
    let x0 = center.0;
    let y0 = center.1;

    while x <= y {
        draw_if_in_bounds(image, x0 + x, y0 + y, color);
        draw_if_in_bounds(image, x0 + y, y0 + x, color);
        draw_if_in_bounds(image, x0 - y, y0 + x, color);
        draw_if_in_bounds(image, x0 - x, y0 + y, color);
        draw_if_in_bounds(image, x0 - x, y0 - y, color);
        draw_if_in_bounds(image, x0 - y, y0 - x, color);
        draw_if_in_bounds(image, x0 + y, y0 - x, color);
        draw_if_in_bounds(image, x0 + x, y0 - y, color);

        x += 1;
        if p < 0 {
            p += 2 * x + 1;
        } else {
            y -= 1;
            p += 2 * (x - y) + 1;
        }
    }
}

/// Draws a cubic Bézier curve, clipped to the image bounds.
pub fn draw_cubic_bezier_curve_mut(
    image: &mut RgbImage,
    start: (f32, f32),
    end: (f32, f32),
    control_a: (f32, f32),
    control_b: (f32, f32),
    color: Rgb<u8>,
) {
    // Bezier curve function from: https://pomax.github.io/bezierinfo/#control
    let cubic_bezier_curve = |t: f32| {
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;
        let x = (start.0 * mt3)
            + (3.0 * control_a.0 * mt2 * t)
            + (3.0 * control_b.0 * mt * t2)
            + (end.0 * t3);
        let y = (start.1 * mt3)
            + (3.0 * control_a.1 * mt2 * t)
            + (3.0 * control_b.1 * mt * t2)
            + (end.1 * t3);
        (x.round(), y.round()) // round to nearest pixel, to avoid ugly line artifacts
    };

    let distance = |point_a: (f32, f32), point_b: (f32, f32)| {
        ((point_a.0 - point_b.0).powi(2) + (point_a.1 - point_b.1).powi(2)).sqrt()
    };

    // Approximate the curve's length by adding the distance between control points.
    let curve_length_bound: f32 =
        distance(start, control_a) + distance(control_a, control_b) + distance(control_b, end);

    // Use a hyperbola to give shorter curves a bias in number of line segments.
    let num_segments: i32 = ((curve_length_bound.powi(2) + 800.0).sqrt() / 8.0) as i32;

    // Sample points along the curve and connect them with line segments.
    let t_interval = 1f32 / (num_segments as f32);
    let mut t1 = 0f32;
    for i in 0..num_segments {
        let t2 = (i as f32 + 1.0) * t_interval;
        draw_line_segment_mut(image, cubic_bezier_curve(t1), cubic_bezier_curve(t2), color);
        t1 = t2;
    }
}

/// Walks the glyphs of `text`, invoking `f` with each outline and its bounds.
///
/// Upstream also returned the laid-out size, for a `text_size` helper the
/// renderer does not use; the advance accumulator it needs is kept.
#[cfg(test)]
fn layout_glyphs(
    scale: PxScale,
    font: &impl Font,
    text: &str,
    mut f: impl FnMut(ab_glyph::OutlinedGlyph, ab_glyph::Rect),
) {
    if text.is_empty() {
        return;
    }
    let scaled = font.as_scaled(scale);

    let mut w = 0.0;
    let mut prev: Option<ab_glyph::GlyphId> = None;

    for c in text.chars() {
        let glyph_id = scaled.glyph_id(c);
        let glyph = glyph_id.with_scale_and_position(scale, point(w, scaled.ascent()));
        w += scaled.h_advance(glyph_id);
        if let Some(g) = scaled.outline_glyph(glyph) {
            if let Some(prev) = prev {
                w += scaled.kern(glyph_id, prev);
            }
            prev = Some(glyph_id);
            let bb = g.px_bounds();
            f(g, bb);
        }
    }
}

/// Draws `text` at `(x, y)`, antialiasing each glyph against the background.
///
/// `Rgb<u8>` has no alpha channel, so upstream's `HAS_ALPHA` branch is dead
/// here and only the `weighted_sum` path is ported.
///
/// **Test-only, and deliberately kept.** The renderer draws through
/// [`rasterize_char`] + [`composite_mask`] instead, so it can deform letters
/// individually. This is the implementation that was verified byte-identical
/// against `imageproc` and is still pinned by
/// `test_geometry_matches_the_digests_verified_against_imageproc`; keeping it
/// gives the mask path an independent oracle to be checked against, which is
/// what `test_the_mask_path_matches_draw_text_mut` does. Deleting it would
/// leave the production path pinned only to itself.
#[cfg(test)]
pub fn draw_text_mut(
    image: &mut RgbImage,
    color: Rgb<u8>,
    x: i32,
    y: i32,
    scale: f32,
    font: &impl Font,
    text: &str,
) {
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;

    layout_glyphs(PxScale::from(scale), font, text, |g, bb| {
        let x_shift = x + bb.min.x.round() as i32;
        let y_shift = y + bb.min.y.round() as i32;
        g.draw(|gx, gy, gv| {
            let image_x = gx as i32 + x_shift;
            let image_y = gy as i32 + y_shift;

            if (0..image_width).contains(&image_x) && (0..image_height).contains(&image_y) {
                let (image_x, image_y) = (image_x as u32, image_y as u32);
                let pixel = *image.get_pixel(image_x, image_y);
                let gv = gv.clamp(0.0, 1.0);
                image.put_pixel(image_x, image_y, weighted_sum(pixel, color, 1.0 - gv, gv));
            }
        })
    });
}

/// A rasterized glyph held as coverage rather than colour.
///
/// [`draw_text_mut`] blends each glyph onto the canvas as it rasterizes, which
/// makes a letter impossible to deform without dragging its background along
/// with it. Rendering to a standalone coverage buffer separates the two: the
/// mask can be warped, moved and scaled on its own, and only then composited.
///
/// Warping coverage rather than rendered pixels also avoids pulling background
/// colour into the glyph edges, and keeps the blend itself on exactly the
/// [`weighted_sum`] path the digests already pin.
///
/// `left` and `top` are the offset of the buffer's top-left corner from the
/// pen position, matching the `x_shift`/`y_shift` [`draw_text_mut`] applies.
#[derive(Clone, Debug)]
pub struct GlyphMask {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    /// Row-major, `width * height` entries, each in `0.0..=1.0`.
    pub coverage: Vec<f32>,
}

impl GlyphMask {
    /// Coverage at integer coordinates, treating everything outside as empty.
    fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0.0;
        }
        self.coverage[y as usize * self.width as usize + x as usize]
    }

    /// Coverage at a fractional `x` on an exact row, linearly interpolated.
    ///
    /// Only `x` needs resampling: every deformation here displaces pixels
    /// horizontally as a function of the row, so `y` stays an exact integer
    /// and a full bilinear filter would only add blur.
    fn sample_row(&self, x: f32, y: i32) -> f32 {
        let floor = x.floor();
        let frac = x - floor;
        let x0 = floor as i32;
        self.at(x0, y) * (1.0 - frac) + self.at(x0 + 1, y) * frac
    }

    /// Slides each row sideways by `displacement(row)`, resampling as it goes.
    ///
    /// The buffer grows to fit the result rather than clipping it, and `left`
    /// moves to match, so the caller composites at the same pen position and
    /// the glyph simply occupies more room.
    ///
    /// Every deformation in the renderer is a horizontal displacement that
    /// depends only on the row — a shear is linear in `y`, a wave is
    /// sinusoidal in `y` — so they share this one function, and a caller that
    /// wants both should sum them into a single closure. Applying two passes
    /// would resample twice and visibly soften the glyph.
    pub fn displace_rows(&self, displacement: impl Fn(f32) -> f32) -> GlyphMask {
        if self.height == 0 || self.width == 0 {
            return self.clone();
        }

        let offsets: Vec<f32> = (0..self.height).map(|y| displacement(y as f32)).collect();
        let min = offsets.iter().copied().fold(f32::INFINITY, f32::min);
        let max = offsets.iter().copied().fold(f32::NEG_INFINITY, f32::max);

        // A row moving left needs room on the left, and vice versa. One extra
        // column on each side absorbs the interpolation reaching past the
        // integer offset.
        let pad_left = (-min).max(0.0).ceil() as u32 + 1;
        let pad_right = max.max(0.0).ceil() as u32 + 1;
        let width = self.width + pad_left + pad_right;

        let mut coverage = vec![0.0f32; (width * self.height) as usize];
        for y in 0..self.height {
            let shift = offsets[y as usize];
            for x in 0..width {
                let value = self.sample_row(x as f32 - pad_left as f32 - shift, y as i32);
                if value > 0.0 {
                    coverage[(y * width + x) as usize] = value;
                }
            }
        }

        GlyphMask {
            width,
            height: self.height,
            left: self.left - pad_left as i32,
            top: self.top,
            coverage,
        }
    }
}

/// Rasterizes a single character into its own coverage buffer.
///
/// Returns `None` for a character with no outline — whitespace, or a glyph
/// missing from the subset font.
pub fn rasterize_char(font: &impl Font, ch: char, scale: PxScale) -> Option<GlyphMask> {
    let scaled = font.as_scaled(scale);
    let glyph_id = scaled.glyph_id(ch);
    let glyph = glyph_id.with_scale_and_position(scale, point(0.0, scaled.ascent()));
    let outlined = scaled.outline_glyph(glyph)?;

    let bounds = outlined.px_bounds();
    let width = (bounds.max.x - bounds.min.x).ceil().max(0.0) as u32;
    let height = (bounds.max.y - bounds.min.y).ceil().max(0.0) as u32;
    if width == 0 || height == 0 {
        return None;
    }

    let mut coverage = vec![0.0f32; (width * height) as usize];
    outlined.draw(|gx, gy, gv| {
        if gx < width && gy < height {
            coverage[(gy * width + gx) as usize] = gv.clamp(0.0, 1.0);
        }
    });

    Some(GlyphMask {
        width,
        height,
        left: bounds.min.x.round() as i32,
        top: bounds.min.y.round() as i32,
        coverage,
    })
}

/// Blends a coverage mask onto the canvas at `(x, y)` in `color`.
///
/// Equivalent to [`draw_text_mut`] for a single character when the mask comes
/// straight from [`rasterize_char`] — `test_the_mask_path_matches_draw_text_mut`
/// holds the two together.
pub fn composite_mask(image: &mut RgbImage, mask: &GlyphMask, x: i32, y: i32, color: Rgb<u8>) {
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;

    for gy in 0..mask.height {
        for gx in 0..mask.width {
            let gv = mask.coverage[(gy * mask.width + gx) as usize];
            // Zero coverage is a no-op: weighted_sum(p, c, 1.0, 0.0) returns p
            // exactly, so skipping it is identical, not merely close.
            if gv <= 0.0 {
                continue;
            }

            let image_x = gx as i32 + x + mask.left;
            let image_y = gy as i32 + y + mask.top;

            if (0..image_width).contains(&image_x) && (0..image_height).contains(&image_y) {
                let (image_x, image_y) = (image_x as u32, image_y as u32);
                let pixel = *image.get_pixel(image_x, image_y);
                let gv = gv.clamp(0.0, 1.0);
                image.put_pixel(image_x, image_y, weighted_sum(pixel, color, 1.0 - gv, gv));
            }
        }
    }
}

/// A standard normal sample, via the Box-Muller transform.
///
/// `u1` is redrawn on an exact zero, which `ln` would send to negative infinity.
fn standard_normal(rng: &mut SmallRng) -> f64 {
    let u1 = loop {
        let u: f64 = rng.random_range(0.0..1.0);
        if u > 0.0 {
            break u;
        }
    };
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Adds independent Gaussian noise to every channel of every pixel.
///
/// Upstream drew from `rand_distr::Normal` over a `rand` 0.9 `StdRng`. Keeping
/// that would have meant carrying `rand_distr` and a second major version of
/// `rand`, so this samples Box-Muller from the `rand` 0.10 already in the tree.
/// A given seed therefore produces a different noise field than `imageproc`
/// would have — which is unobservable, because every seed is itself drawn
/// freshly per CAPTCHA and no output is reproducible across calls anyway. The
/// distribution, and so the visual result, is the same.
pub fn gaussian_noise_mut(image: &mut RgbImage, mean: f64, stddev: f64, seed: u64) {
    let mut rng = SmallRng::seed_from_u64(seed);

    for pixel in image.pixels_mut() {
        for channel in pixel.0.iter_mut() {
            let noise = mean + stddev * standard_normal(&mut rng);
            *channel = clamp_u8_f64(f64::from(*channel) + noise);
        }
    }
}

/// Speckles pixels at the given `rate`, alternating between very light and
/// very dark, at a random hue each time.
///
/// Upstream — and this port until the palette was randomised — set the speck to
/// pure black or pure white. That was a segmentation key: a solver could
/// identify every speck by testing for exactly `[0,0,0]` or `[255,255,255]` and
/// drop them all before looking at the glyphs. Randomising the hue keeps the
/// high-contrast character that makes the noise worth having while leaving
/// nothing exact to test for.
///
/// Samples from `rand` 0.10 rather than `rand_distr`, for the reason given on
/// [`gaussian_noise_mut`].
pub fn salt_and_pepper_noise_mut(image: &mut RgbImage, rate: f64, seed: u64) {
    let mut rng = SmallRng::seed_from_u64(seed);

    for pixel in image.pixels_mut() {
        if rng.random_range(0.0..1.0) > rate {
            continue;
        }
        let hue: f32 = rng.random_range(0.0..1.0);
        let saturation: f32 = rng.random_range(0.35..1.0);
        // Still salt *and* pepper: an even split between the light and dark
        // ends, just no longer at the exact extremes of the range.
        let lightness: f32 = if rng.random_range(0.0..1.0) >= 0.5 {
            rng.random_range(0.82..1.0)
        } else {
            rng.random_range(0.0..0.18)
        };
        *pixel = hsl_to_rgb(hue, saturation, lightness);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::FontArc;
    use sha2::{Digest, Sha256};

    fn font() -> FontArc {
        FontArc::try_from_slice(include_bytes!(
            "../../../assets/fonts/Roboto-Bold-subset.ttf"
        ))
        .unwrap()
    }

    /// A background with structure in it, so a regression that shifts or drops
    /// pixels cannot hide behind a uniform fill.
    fn canvas() -> RgbImage {
        RgbImage::from_fn(220, 120, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 90]))
    }

    fn digest(image: &RgbImage) -> String {
        let mut hasher = Sha256::new();
        hasher.update(image.as_raw());
        hex::encode(hasher.finalize())[..16].to_string()
    }

    /// Pins the rendered output of each geometry primitive.
    ///
    /// These digests were captured from this implementation while it was still
    /// running side by side with `imageproc` 0.26.2, under a differential test
    /// asserting the two produced byte-identical buffers over a range of
    /// inputs (glyphs with descenders, curves running off-canvas, circles
    /// clipped at the corners, degenerate radii). They therefore encode
    /// upstream's behaviour, not merely this port's — which is what makes them
    /// worth keeping now that the dependency is gone and the comparison can no
    /// longer be re-run from the tree.
    #[test]
    fn test_geometry_matches_the_digests_verified_against_imageproc() {
        let mut text = canvas();
        draw_text_mut(&mut text, Rgb([214, 14, 50]), 5, 45, 35.0, &font(), "gjQ");
        assert_eq!(
            digest(&text),
            "ad2ecaf94f66d31f",
            "draw_text_mut output changed"
        );

        let mut bezier = canvas();
        draw_cubic_bezier_curve_mut(
            &mut bezier,
            (5.0, 20.0),
            (215.0, 90.0),
            (60.0, 10.0),
            (160.0, 115.0),
            Rgb([240, 181, 41]),
        );
        assert_eq!(digest(&bezier), "aae1fcd222a9524b", "bezier output changed");

        let mut circle = canvas();
        draw_hollow_circle_mut(&mut circle, (50, 50), 12, Rgb([176, 203, 40]));
        assert_eq!(digest(&circle), "e8c8641ca34e34b8", "circle output changed");
    }

    /// The mask path must be a drop-in for `draw_text_mut` before any
    /// deformation is applied, or the digests above stop meaning anything for
    /// the renderer that now goes through masks.
    ///
    /// Covers descenders, round glyphs, the widest and narrowest letters in the
    /// subset, three scales, and positions that clip on every edge.
    #[test]
    fn test_the_mask_path_matches_draw_text_mut() {
        let font = font();
        let color = Rgb([214, 14, 50]);

        for ch in ['g', 'j', 'Q', 'W', 'm', 'z', '2', '9', 'x'] {
            for scale in [35.0f32, 42.0, 50.0] {
                for (x, y) in [(5, 45), (0, 0), (200, 100), (-12, -12), (215, 115)] {
                    let mut direct = canvas();
                    let mut viamask = canvas();

                    let mut buf = [0u8; 4];
                    draw_text_mut(
                        &mut direct,
                        color,
                        x,
                        y,
                        scale,
                        &font,
                        ch.encode_utf8(&mut buf),
                    );

                    let mask = rasterize_char(&font, ch, PxScale::from(scale))
                        .expect("subset glyphs all have outlines");
                    composite_mask(&mut viamask, &mask, x, y, color);

                    assert_eq!(
                        direct, viamask,
                        "mask path diverges for {ch:?} at scale {scale} pos ({x},{y})"
                    );
                }
            }
        }
    }

    fn glyph() -> GlyphMask {
        rasterize_char(&font(), 'K', PxScale::from(50.0)).expect("K is in the subset")
    }

    /// Total coverage — the glyph's "ink". Displacement moves ink around but
    /// must not create or destroy much of it.
    fn ink(mask: &GlyphMask) -> f32 {
        mask.coverage.iter().sum()
    }

    #[test]
    fn test_zero_displacement_leaves_the_glyph_where_it_was() {
        let original = glyph();
        let same = original.displace_rows(|_| 0.0);

        // The buffer gains its one-column interpolation margin on each side,
        // and `left` moves to match, so the glyph lands in the same place.
        assert_eq!(same.width, original.width + 2);
        assert_eq!(same.left, original.left - 1);
        assert_eq!(same.height, original.height);

        for y in 0..original.height as i32 {
            for x in 0..original.width as i32 {
                assert!(
                    (same.at(x + 1, y) - original.at(x, y)).abs() < 1e-5,
                    "coverage changed at ({x},{y}) under a zero displacement"
                );
            }
        }
    }

    #[test]
    fn test_a_constant_displacement_is_a_pure_translation() {
        let original = glyph();
        let moved = original.displace_rows(|_| 4.0);

        for y in 0..original.height as i32 {
            for x in 0..original.width as i32 {
                assert!(
                    (moved.at(x + 1 + 4, y) - original.at(x, y)).abs() < 1e-5,
                    "a constant shift should move ink without reshaping it, at ({x},{y})"
                );
            }
        }
        assert!(
            (ink(&moved) - ink(&original)).abs() < 0.5,
            "translation should conserve ink: {} vs {}",
            ink(&moved),
            ink(&original)
        );
    }

    #[test]
    fn test_a_shear_leans_the_glyph_without_losing_ink() {
        let original = glyph();
        let centre = original.height as f32 / 2.0;
        let sheared = original.displace_rows(|y| 0.4 * (y - centre));

        assert!(
            sheared.width > original.width,
            "a shear must widen the buffer: {} vs {}",
            sheared.width,
            original.width
        );

        // Interpolation redistributes coverage but should not consume it.
        let (before, after) = (ink(&original), ink(&sheared));
        assert!(
            (after - before).abs() / before < 0.02,
            "shear lost or invented ink: {before} -> {after}"
        );

        // Top and bottom rows must end up displaced in opposite directions.
        let row_centroid = |mask: &GlyphMask, y: i32| -> Option<f32> {
            let mut weight = 0.0;
            let mut moment = 0.0;
            for x in 0..mask.width as i32 {
                let v = mask.at(x, y);
                weight += v;
                moment += v * (x as f32 + mask.left as f32);
            }
            (weight > 0.1).then(|| moment / weight)
        };

        let top = 1;
        let bottom = original.height as i32 - 2;
        let (ot, ob) = (
            row_centroid(&original, top).unwrap(),
            row_centroid(&original, bottom).unwrap(),
        );
        let (st, sb) = (
            row_centroid(&sheared, top).unwrap(),
            row_centroid(&sheared, bottom).unwrap(),
        );
        assert!(
            st < ot && sb > ob,
            "shear should pull the top left and the bottom right: \
             top {ot}->{st}, bottom {ob}->{sb}"
        );
    }

    /// The property that separates a wave from a shear: a shear displaces rows
    /// monotonically from top to bottom, a sine sends them back the other way.
    #[test]
    fn test_a_sine_displacement_bends_rows_in_both_directions() {
        let original = glyph();
        let height = original.height as f32;
        let waved = original.displace_rows(|y| 6.0 * (std::f32::consts::TAU * y / height).sin());

        let (before, after) = (ink(&original), ink(&waved));
        assert!(
            (after - before).abs() / before < 0.02,
            "the wave lost or invented ink: {before} -> {after}"
        );

        let centroid = |mask: &GlyphMask, y: i32| -> Option<f32> {
            let mut weight = 0.0;
            let mut moment = 0.0;
            for x in 0..mask.width as i32 {
                let v = mask.at(x, y);
                weight += v;
                moment += v * (x as f32 + mask.left as f32);
            }
            (weight > 0.1).then(|| moment / weight)
        };

        // One full cycle over the glyph's height pushes the upper rows one way
        // and the lower rows the other.
        let mut shifts = Vec::new();
        for y in 0..original.height as i32 {
            if let (Some(a), Some(b)) = (centroid(&original, y), centroid(&waved, y)) {
                shifts.push(b - a);
            }
        }
        assert!(shifts.len() > 4, "not enough inked rows to judge");
        assert!(
            shifts.iter().any(|s| *s > 1.0) && shifts.iter().any(|s| *s < -1.0),
            "a sine must displace rows both left and right; shifts were {shifts:?}"
        );
    }

    /// A character missing from the subset does not vanish — it maps to
    /// `.notdef`, which in this font is a visible box. That is precisely what
    /// makes the generator's `test_every_basic_char_has_a_glyph` worth having:
    /// a stale subset would show boxes to users, not blanks.
    #[test]
    fn test_a_missing_glyph_rasterizes_to_the_notdef_box() {
        let font = font();
        let scale = PxScale::from(40.0);

        let notdef = rasterize_char(&font, '1', scale).expect(".notdef is outlined in this font");
        assert!(notdef.width > 0 && notdef.height > 0);

        for ch in ['O', '@', ' '] {
            let other = rasterize_char(&font, ch, scale).expect("also .notdef");
            assert_eq!(
                (other.width, other.height),
                (notdef.width, notdef.height),
                "{ch:?} should resolve to the same .notdef box"
            );
        }

        // A real glyph must differ, or the equality above proves nothing.
        let real = rasterize_char(&font, 'Q', scale).expect("Q is in the subset");
        assert_ne!((real.width, real.height), (notdef.width, notdef.height));
    }

    /// `imageproc::pixelops::weighted_sum`'s own documented example. Independent
    /// of the digests above: it pins the truncating clamp that decides every
    /// antialiased glyph edge, and would catch a switch to rounding.
    #[test]
    fn test_weighted_sum_matches_upstreams_documented_example() {
        let left = Rgb([10u8, 20u8, 30u8]);
        let right = Rgb([100u8, 80u8, 60u8]);
        assert_eq!(weighted_sum(left, right, 0.7, 0.3), Rgb([37, 38, 39]));
    }

    #[test]
    fn test_hsl_to_rgb_matches_known_colours() {
        // Primaries at full saturation and mid lightness.
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), Rgb([255, 0, 0]));
        assert_eq!(hsl_to_rgb(1.0 / 3.0, 1.0, 0.5), Rgb([0, 255, 0]));
        assert_eq!(hsl_to_rgb(2.0 / 3.0, 1.0, 0.5), Rgb([0, 0, 255]));

        // Zero saturation is grey whatever the hue, and the extremes of
        // lightness are black and white.
        for hue in [0.0, 0.25, 0.5, 0.9] {
            assert_eq!(hsl_to_rgb(hue, 0.0, 0.5), Rgb([128, 128, 128]));
            assert_eq!(hsl_to_rgb(hue, 1.0, 0.0), Rgb([0, 0, 0]));
            assert_eq!(hsl_to_rgb(hue, 1.0, 1.0), Rgb([255, 255, 255]));
        }

        // Hue wraps rather than clipping, so a caller can pass any turn count.
        assert_eq!(hsl_to_rgb(1.0, 1.0, 0.5), hsl_to_rgb(0.0, 1.0, 0.5));
        assert_eq!(hsl_to_rgb(-0.25, 0.8, 0.4), hsl_to_rgb(0.75, 0.8, 0.4));
    }

    /// Sweeping the hue must produce a genuinely continuous range of colours,
    /// not cluster on a few values — that continuity is what removed the
    /// palette as a segmentation key.
    #[test]
    fn test_hue_sweep_covers_the_colour_wheel() {
        let colours: std::collections::HashSet<[u8; 3]> = (0..360)
            .map(|d| hsl_to_rgb(d as f32 / 360.0, 0.8, 0.45).0)
            .collect();
        assert!(
            colours.len() > 300,
            "a 360-step hue sweep should give hundreds of distinct colours, got {}",
            colours.len()
        );
    }

    #[test]
    fn test_clamp_truncates_and_saturates() {
        assert_eq!(clamp_u8_f32(37.999), 37, "clamp truncates, never rounds");
        assert_eq!(clamp_u8_f32(-4.0), 0);
        assert_eq!(clamp_u8_f32(1e9), 255);
        assert_eq!(clamp_u8_f32(f32::NAN), 255, "NaN fails both compares");
        assert_eq!(clamp_u8_f64(254.9), 254);
        assert_eq!(clamp_u8_f64(-0.5), 0);
        assert_eq!(clamp_u8_f64(300.0), 255);
    }

    /// Every primitive clips rather than panicking, which is what keeps the
    /// renderer safe on the smallest canvas it will accept.
    #[test]
    fn test_drawing_off_canvas_clips_instead_of_panicking() {
        let mut image = RgbImage::from_pixel(30, 20, Rgb([1, 2, 3]));
        let color = Rgb([9, 9, 9]);

        draw_hollow_circle_mut(&mut image, (-40, -40), 15, color);
        draw_hollow_circle_mut(&mut image, (500, 500), 15, color);
        draw_hollow_circle_mut(&mut image, (15, 10), 0, color);
        draw_cubic_bezier_curve_mut(
            &mut image,
            (-100.0, -100.0),
            (900.0, 900.0),
            (-50.0, 400.0),
            (400.0, -50.0),
            color,
        );
        draw_line_segment_mut(&mut image, (-10.0, -10.0), (999.0, 999.0), color);
        draw_text_mut(&mut image, color, -60, -60, 50.0, &font(), "Q");
        draw_text_mut(&mut image, color, 400, 400, 50.0, &font(), "Q");
        draw_text_mut(&mut image, color, 5, 5, 35.0, &font(), "");

        assert_eq!(image.dimensions(), (30, 20));
    }

    #[test]
    fn test_a_zero_radius_circle_marks_only_its_centre() {
        let mut image = RgbImage::from_pixel(9, 9, Rgb([0, 0, 0]));
        draw_hollow_circle_mut(&mut image, (4, 4), 0, Rgb([255, 255, 255]));
        let lit = image.pixels().filter(|p| p.0 == [255, 255, 255]).count();
        assert_eq!(lit, 1, "r=0 should touch exactly the centre pixel");
        assert_eq!(image.get_pixel(4, 4).0, [255, 255, 255]);
    }

    #[test]
    fn test_gaussian_noise_tracks_the_requested_distribution() {
        let mut image = RgbImage::from_pixel(200, 200, Rgb([128, 128, 128]));
        gaussian_noise_mut(&mut image, 4.0, 20.0, 42);

        let values: Vec<f64> = image.pixels().flat_map(|p| p.0).map(f64::from).collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let stddev =
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt();

        // Centred on 128 + mean, spread by stddev. Loose bounds: this is a
        // sampled distribution, and clamping at 0/255 pulls both in slightly.
        assert!((mean - 132.0).abs() < 1.0, "mean was {mean}, expected ~132");
        assert!(
            (stddev - 20.0).abs() < 1.5,
            "stddev was {stddev}, expected ~20"
        );
    }

    /// Identifies specks by "not the background" rather than by exact value,
    /// which is the whole point of the change: there is no longer an exact
    /// value to test for. Lightness still splits evenly between the light and
    /// dark ends, so it remains salt *and* pepper.
    #[test]
    fn test_salt_and_pepper_hits_the_requested_rate_at_both_ends() {
        let ground = Rgb([128, 128, 128]);
        let mut image = RgbImage::from_pixel(200, 200, ground);
        salt_and_pepper_noise_mut(&mut image, 0.05, 7);

        let specks: Vec<&Rgb<u8>> = image.pixels().filter(|p| **p != ground).collect();
        let rate = specks.len() as f64 / 40_000.0;
        assert!(
            (rate - 0.05).abs() < 0.01,
            "rate was {rate}, expected ~0.05"
        );

        let luma = |p: &Rgb<u8>| {
            0.2126 * f64::from(p.0[0]) + 0.7152 * f64::from(p.0[1]) + 0.0722 * f64::from(p.0[2])
        };
        let salt = specks.iter().filter(|p| luma(p) > 128.0).count();
        let pepper = specks.len() - salt;
        assert!(salt > 0 && pepper > 0, "both ends should appear");
        let skew = (salt as f64 - pepper as f64).abs() / specks.len() as f64;
        assert!(
            skew < 0.2,
            "the two ends should be near-equally likely, skew {skew}"
        );

        // The key that was removed: specks must not all sit on a handful of
        // exact values a solver could enumerate.
        let distinct: std::collections::HashSet<[u8; 3]> = specks.iter().map(|p| p.0).collect();
        assert!(
            distinct.len() > specks.len() / 2,
            "specks should be individually coloured, not drawn from a small set: \
             {} distinct across {} specks",
            distinct.len(),
            specks.len()
        );
    }

    #[test]
    fn test_noise_is_reproducible_for_a_seed_and_varies_across_seeds() {
        let base = RgbImage::from_pixel(64, 64, Rgb([128, 128, 128]));

        let mut a = base.clone();
        let mut b = base.clone();
        gaussian_noise_mut(&mut a, 0.0, 15.0, 99);
        gaussian_noise_mut(&mut b, 0.0, 15.0, 99);
        assert_eq!(a, b, "the same seed must give the same field");

        let mut c = base.clone();
        gaussian_noise_mut(&mut c, 0.0, 15.0, 100);
        assert_ne!(a, c, "a different seed must give a different field");

        let mut d = base.clone();
        let mut e = base.clone();
        salt_and_pepper_noise_mut(&mut d, 0.1, 5);
        salt_and_pepper_noise_mut(&mut e, 0.1, 5);
        assert_eq!(d, e, "salt and pepper must be reproducible too");
    }

    #[test]
    fn test_a_zero_rate_leaves_the_image_untouched() {
        let base = RgbImage::from_pixel(32, 32, Rgb([128, 128, 128]));
        let mut image = base.clone();
        salt_and_pepper_noise_mut(&mut image, 0.0, 1);
        assert_eq!(image, base);
    }
}
