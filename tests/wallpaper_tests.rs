use image::ImageFormat;

use marktype::settings::ThemePreset;
use marktype::wallpaper::fx::{self, Effect};
use marktype::wallpaper::gallery::BUILTINS;
use marktype::wallpaper::panel::{self, MAX_ALPHA, MIN_ALPHA, Panel};
use marktype::wallpaper::theme_tint::{FAINT_CONTRAST, Palette, TEXT_CONTRAST};

fn solid_rgba(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    (0..w * h)
        .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
        .collect()
}

#[test]
fn all_seven_kite_builtin_wallpapers_are_valid_jpegs() {
    assert_eq!(BUILTINS.len(), 7);
    for b in &BUILTINS {
        assert_eq!(
            image::guess_format(b.bytes).unwrap(),
            ImageFormat::Jpeg,
            "builtin {} should be valid JPEG",
            b.id
        );
    }
}

#[test]
fn pixel_effects_preserve_size_and_transform_output() {
    let src: Vec<u8> = (0..32 * 24)
        .flat_map(|i| [(i % 255) as u8, (i * 3 % 255) as u8, 200, 255])
        .collect();
    for effect in [
        Effect::Dither,
        Effect::Ascii,
        Effect::Halftone,
        Effect::Scanlines,
    ] {
        for light in [false, true] {
            let out = fx::render(src.clone(), 32, 24, effect, light);
            assert_eq!(out.len(), src.len());
            assert_ne!(out, src);
        }
    }
    assert_eq!(fx::render(src.clone(), 32, 24, Effect::None, false), src);
}

#[test]
fn dominant_color_extraction_favors_chromatic_regions() {
    let mut pixels = vec![[128, 128, 128, 255]; 100];
    pixels.extend(vec![[0, 0, 255, 255]; 30]);
    assert_eq!(fx::extract(pixels), Some([0, 0, 255]));

    let scan = fx::render(
        solid_rgba(3, 3, [200, 100, 50]),
        3,
        3,
        Effect::Scanlines,
        false,
    );
    assert_eq!(&scan[0..3], &[104, 52, 26]);
}

#[test]
fn panel_alpha_scales_with_backdrop_luminance() {
    let panel_cfg = Panel {
        surface: [0x0d, 0x0d, 0x0d],
        texts: &[([0xe8, 0xe8, 0xea], 4.5), ([0xa9, 0xa9, 0xae], 4.5)],
    };
    let on_black = panel::panel_alpha(&[[0, 0, 0]; 21], &[panel_cfg]);
    assert_eq!(on_black, MIN_ALPHA);

    let panel_cfg2 = Panel {
        surface: [0x0d, 0x0d, 0x0d],
        texts: &[([0xe8, 0xe8, 0xea], 4.5), ([0xa9, 0xa9, 0xae], 4.5)],
    };
    let on_white = panel::panel_alpha(&[[255, 255, 255]; 21], &[panel_cfg2]);
    assert!((0.80..=MAX_ALPHA).contains(&on_white));
}

#[test]
fn wallpaper_tinting_preserves_wcag_contrast_in_all_modes() {
    let test_colors = [[0, 0, 0], [255, 255, 255], [250, 210, 30], [20, 60, 220]];
    for preset in [
        ThemePreset::Kite,
        ThemePreset::Github,
        ThemePreset::Nord,
        ThemePreset::Sepia,
    ] {
        for light in [false, true] {
            for color in test_colors {
                let tinted = Palette::base(preset, light).tinted(color);
                for surface in [
                    tinted.shell,
                    tinted.editor_bg,
                    tinted.preview_bg,
                    tinted.code_bg,
                ] {
                    let s_rgb = fx::pack(surface);
                    assert!(
                        fx::contrast_ratio(fx::pack(tinted.text), s_rgb) >= TEXT_CONTRAST - 0.05
                    );
                    assert!(
                        fx::contrast_ratio(fx::pack(tinted.muted), s_rgb) >= TEXT_CONTRAST - 0.05
                    );
                    assert!(
                        fx::contrast_ratio(fx::pack(tinted.faint), s_rgb) >= FAINT_CONTRAST - 0.05
                    );
                }
            }
        }
    }
}

#[test]
fn fx_render_guards_against_u32_dimension_overflow() {
    let input = vec![128u8; 16];
    let out = fx::render(input.clone(), 65_536, 16_384, Effect::Halftone, false);
    assert_eq!(out, input);
}
