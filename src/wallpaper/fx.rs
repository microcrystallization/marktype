// Adapted from Kite (https://github.com/Kfkcome/kite) & zeron (MIT).
//! Wallpaper pixel work: artwork effects (None, Scanlines, ASCII, Halftone, Dither),
//! quantized dominant chromatic colour extraction, WCAG contrast ratio, and contrast-guarded accent.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    #[default]
    None,
    Dither,
    Ascii,
    Halftone,
    Scanlines,
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((2126 * r as u32 + 7152 * g as u32 + 722 * b as u32) / 10000) as u8
}

struct Source<'a> {
    w: u32,
    h: u32,
    rgba: &'a [u8],
}

impl Source<'_> {
    fn color(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    fn luma(&self, x: u32, y: u32) -> u8 {
        let [r, g, b, _] = self.color(x, y);
        luma(r, g, b)
    }
}

/// Apply `effect` to an RGBA image; `light` is the appearance it's drawn in
/// (effects print on white paper in light mode, black in dark).
pub fn render(rgba: Vec<u8>, width: u32, height: u32, effect: Effect, light: bool) -> Vec<u8> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    let Some(expected) = expected else {
        return rgba;
    };
    if width == 0 || height == 0 || rgba.len() < expected {
        return rgba;
    }
    let src = Source {
        w: width,
        h: height,
        rgba: &rgba[..expected],
    };
    match effect {
        Effect::None => rgba,
        Effect::Scanlines => scanlines(&src, light),
        Effect::Ascii => ascii(&src, light),
        Effect::Halftone => halftone(&src, light),
        Effect::Dither => dither(&src),
    }
}

fn scanlines(s: &Source, light: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.rgba.len());
    for y in 0..s.h {
        let gain = if y % 3 == 0 { 0.52 } else { 1.0 };
        for x in 0..s.w {
            let [r, g, b, a] = s.color(x, y);
            let ch = |v: u8| {
                if light {
                    (v as f32 + (255.0 - v as f32) * (1.0 - gain)) as u8
                } else {
                    (v as f32 * gain) as u8
                }
            };
            out.extend_from_slice(&[ch(r), ch(g), ch(b), a]);
        }
    }
    out
}

fn ascii(s: &Source, light: bool) -> Vec<u8> {
    const GLYPHS: [[u8; 7]; 10] = [
        [0, 0, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 0, 4, 0],
        [0, 4, 0, 0, 4, 0, 0],
        [0, 0, 0, 14, 0, 0, 0],
        [0, 0, 14, 0, 14, 0, 0],
        [0, 4, 4, 31, 4, 4, 0],
        [0, 21, 14, 31, 14, 21, 0],
        [10, 10, 31, 10, 31, 10, 10],
        [17, 2, 4, 4, 8, 16, 17],
        [14, 17, 23, 21, 23, 16, 14],
    ];
    let mut out = Vec::with_capacity(s.rgba.len());
    for y in 0..s.h {
        for x in 0..s.w {
            let sx = (x / 6 * 6 + 3).min(s.w - 1);
            let sy = (y / 8 * 8 + 4).min(s.h - 1);
            let l = s.luma(sx, sy);
            let density = if light { 255 - l } else { l };
            let index = ((density as f32 / 255.0).sqrt() * 9.0) as usize;
            let ink = x % 6 < 5
                && y % 8 < 7
                && GLYPHS[index.min(9)][y as usize % 8] & (1 << (4 - x % 6)) != 0;
            let [r, g, b, a] = s.color(x, y);
            let [cr, cg, cb, _] = s.color(sx, sy);
            let paper = if light { 255.0 } else { 0.0 };
            let mix = |base: u8, glyph: u8| {
                (base as f32 * 0.60
                    + if ink {
                        glyph as f32 * 0.40
                    } else {
                        paper * 0.40
                    }) as u8
            };
            out.extend_from_slice(&[mix(r, cr), mix(g, cg), mix(b, cb), a]);
        }
    }
    out
}

fn halftone(s: &Source, light: bool) -> Vec<u8> {
    let paper = if light { 255u8 } else { 0 };
    let mut out = vec![0u8; s.rgba.len()];
    for px in out.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&[paper, paper, paper, 255]);
    }
    for y in (0..s.h).step_by(4) {
        for x in (0..s.w).step_by(4) {
            let l = s.luma(x, y);
            let l = if light { 255 - l } else { l };
            let radius = 2.0 * (0.3 + 0.7 * (l as f32 / 255.0).sqrt());
            let [r, g, b, a] = s.color((x + 2).min(s.w - 1), (y + 2).min(s.h - 1));
            for dy in 0..4.min(s.h - y) {
                for dx in 0..4.min(s.w - x) {
                    let distance = ((dx as f32 - 1.5).powi(2) + (dy as f32 - 1.5).powi(2)).sqrt();
                    let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0) * a as f32 / 255.0;
                    let [sr, sg, sb, sa] = s.color(x + dx, y + dy);
                    let blend = |source: u8, dot: u8| {
                        (source as f32 * 0.60
                            + (dot as f32 * coverage + paper as f32 * (1.0 - coverage)) * 0.40)
                            as u8
                    };
                    let i = (((y + dy) * s.w + x + dx) * 4) as usize;
                    out[i..i + 4].copy_from_slice(&[blend(sr, r), blend(sg, g), blend(sb, b), sa]);
                }
            }
        }
    }
    out
}

fn dither(s: &Source) -> Vec<u8> {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let mut out = vec![0u8; s.rgba.len()];
    for y in (0..s.h).step_by(2) {
        for x in (0..s.w).step_by(2) {
            let [r, g, b, a] = s.color((x + 1).min(s.w - 1), (y + 1).min(s.h - 1));
            let threshold = BAYER[y as usize / 2 % 4][x as usize / 2 % 4];
            let peak = r.max(g).max(b) as f32;
            let bright = peak / 255.0 > (threshold as f32 + 0.5) / 16.0;
            let gain = if bright { 255.0 / peak.max(1.0) } else { 0.08 };
            let c = [
                (r as f32 * gain).round() as u8,
                (g as f32 * gain).round() as u8,
                (b as f32 * gain).round() as u8,
                a,
            ];
            for dy in 0..2.min(s.h - y) {
                for dx in 0..2.min(s.w - x) {
                    let i = (((y + dy) * s.w + x + dx) * 4) as usize;
                    out[i..i + 4].copy_from_slice(&c);
                }
            }
        }
    }
    out
}

fn linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn rel_luminance(rgb: u32) -> f32 {
    let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG contrast ratio between two `0xRRGGBB` colours.
pub fn contrast_ratio(a_rgb: u32, b_rgb: u32) -> f32 {
    let a = rel_luminance(a_rgb);
    let b = rel_luminance(b_rgb);
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

pub const ACCENT_CONTRAST: f32 = 3.0;

/// Quantized dominant colour, favouring chromatic regions over neutral pixels.
pub fn extract(pixels: impl IntoIterator<Item = [u8; 4]>) -> Option<[u8; 3]> {
    let mut bins = vec![(0.0_f64, [0.0_f64; 3]); 4096];
    for [r, g, b, a] in pixels {
        if a < 128 {
            continue;
        }
        let high = f64::from(r.max(g).max(b));
        let low = f64::from(r.min(g).min(b));
        let saturation = (high - low) / high.max(1.0);
        let weight = (0.2 + saturation * saturation) * f64::from(a) / 255.0;
        let index =
            ((usize::from(r) >> 4) << 8) | ((usize::from(g) >> 4) << 4) | (usize::from(b) >> 4);
        let (count, channels) = &mut bins[index];
        *count += weight;
        for (sum, channel) in channels.iter_mut().zip([r, g, b]) {
            *sum += f64::from(channel) * weight;
        }
    }
    let (count, channels) = bins.into_iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    (count > 0.0).then(|| channels.map(|c| (c / count).round() as u8))
}

pub fn pack(c: [u8; 3]) -> u32 {
    u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])
}

/// `color` moved toward white (on a dark surface) or black (on a light one), in 5% steps,
/// until it keeps `ACCENT_CONTRAST` against `surface`.
pub fn accent_for(color: [u8; 3], surface: [u8; 3]) -> [u8; 3] {
    let surface_rgb = pack(surface);
    let dark_surface = contrast_ratio(surface_rgb, 0) < contrast_ratio(surface_rgb, 0xFF_FFFF);
    let target = if dark_surface { 255.0 } else { 0.0 };
    for step in 0..=20u8 {
        let t = f32::from(step) * 0.05;
        let mixed = color.map(|c| (f32::from(c) * (1.0 - t) + target * t).round() as u8);
        if contrast_ratio(pack(mixed), surface_rgb) >= ACCENT_CONTRAST {
            return mixed;
        }
    }
    [target as u8; 3]
}
