// Adapted from Kite (https://github.com/Kfkcome/kite) wallpaper_panel.rs.
//! Computes the smallest panel opacity (`MIN_ALPHA..=MAX_ALPHA`) at which every text colour
//! keeps its WCAG contrast over the wallpaper's darkest through brightest regions.

use super::fx::{contrast_ratio, pack};

pub const MIN_ALPHA: f32 = 0.55;
pub const MAX_ALPHA: f32 = 0.92;
const SAMPLES: usize = 21;
const STEP: f32 = 0.01;

pub struct Panel<'a> {
    pub surface: [u8; 3],
    pub texts: &'a [([u8; 3], f32)],
}

/// 21 colours from darkest to brightest between the 2nd and 98th luminance percentiles.
pub fn backdrop(rgba: &[u8]) -> Vec<[u8; 3]> {
    let mut pixels: Vec<[u8; 3]> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] >= 128)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    pixels.sort_by(|a, b| contrast_ratio(pack(*a), 0).total_cmp(&contrast_ratio(pack(*b), 0)));
    if pixels.is_empty() {
        return Vec::new();
    }
    (0..SAMPLES)
        .map(|i| {
            let p = 0.02 + 0.96 * i as f32 / (SAMPLES - 1) as f32;
            pixels[((pixels.len() - 1) as f32 * p).round() as usize]
        })
        .collect()
}

fn over(surface: [u8; 3], under: [u8; 3], alpha: f32) -> [u8; 3] {
    let ch = |i: usize| {
        (f32::from(surface[i]) * alpha + f32::from(under[i]) * (1.0 - alpha)).round() as u8
    };
    [ch(0), ch(1), ch(2)]
}

fn holds(backdrop: &[[u8; 3]], panels: &[Panel], alpha: f32) -> bool {
    panels.iter().all(|panel| {
        backdrop.iter().all(|under| {
            let seen = pack(over(panel.surface, *under, alpha));
            panel
                .texts
                .iter()
                .all(|(text, minimum)| contrast_ratio(pack(*text), seen) >= *minimum)
        })
    })
}

pub fn panel_alpha(backdrop: &[[u8; 3]], panels: &[Panel]) -> f32 {
    if backdrop.is_empty() {
        return MAX_ALPHA;
    }
    let steps = ((MAX_ALPHA - MIN_ALPHA) / STEP).round() as u32;
    (0..=steps)
        .map(|i| MIN_ALPHA + STEP * i as f32)
        .find(|a| holds(backdrop, panels, *a))
        .unwrap_or(MAX_ALPHA)
}
