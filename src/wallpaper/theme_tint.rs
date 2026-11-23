// Adapted from Kite (https://github.com/Kfkcome/kite) theme.rs & theme_tint.rs.
//! Computes the full adaptive theme palette, wallpaper-derived tinting, WCAG contrast-hardened
//! text colours, and contrast-guarded glass panel opacity.

use serde::Serialize;

use crate::settings::ThemePreset;

use super::fx::{accent_for, contrast_ratio, pack};
use super::panel::{self, Panel};

const DARK_PREMIX: f32 = 0.88;
const LIGHT_PREMIX: f32 = 0.94;
const SURFACE_MIX: f32 = 0.40;
pub const TEXT_CONTRAST: f32 = 4.5;
pub const FAINT_CONTRAST: f32 = 3.0;

#[derive(Debug, Clone)]
pub struct Palette {
    pub light: bool,
    pub shell: [u8; 3],
    pub editor_bg: [u8; 3],
    pub preview_bg: [u8; 3],
    pub popup: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub faint: [u8; 3],
    pub accent: [u8; 3],
    pub on_accent: [u8; 3],
    pub code_bg: [u8; 3],
    pub border_rgba: &'static str,
    pub hairline_rgba: &'static str,
    pub row_hover_rgba: String,
    pub row_active_rgba: String,
    pub tint: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTheme {
    pub light: bool,
    pub shell: String,
    pub shell_panel: String,
    pub editor_bg: String,
    pub editor_panel: String,
    pub preview_bg: String,
    pub preview_panel: String,
    pub popup: String,
    pub text: String,
    pub muted: String,
    pub faint: String,
    pub accent: String,
    pub accent_soft: String,
    pub on_accent: String,
    pub code_bg: String,
    pub border: String,
    pub hairline: String,
    pub row_hover: String,
    pub row_active: String,
    pub panel_alpha: f32,
}

fn unpack(hex: u32) -> [u8; 3] {
    [
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    ]
}

fn hex_str(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn rgba_str(c: [u8; 3], alpha: f32) -> String {
    format!(
        "rgba({}, {}, {}, {:.3})",
        c[0],
        c[1],
        c[2],
        alpha.clamp(0.0, 1.0)
    )
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let ch = |i: usize| (f32::from(a[i]) * (1.0 - t) + f32::from(b[i]) * t).round() as u8;
    [ch(0), ch(1), ch(2)]
}

fn premixed(color: [u8; 3], light: bool) -> [u8; 3] {
    if light {
        mix(color, [255; 3], LIGHT_PREMIX)
    } else {
        mix(color, [0; 3], DARK_PREMIX)
    }
}

fn on_accent_for(accent: [u8; 3]) -> [u8; 3] {
    let rgb = pack(accent);
    if contrast_ratio(rgb, 0xFF_FFFF) >= contrast_ratio(rgb, 0x00_0000) {
        [255, 255, 255]
    } else {
        [15, 15, 18]
    }
}

pub fn harden(
    color: [u8; 3],
    backgrounds: &[[u8; 3]],
    minimum: f32,
    preferred: Option<[u8; 3]>,
) -> [u8; 3] {
    let worst = |c: [u8; 3]| {
        backgrounds
            .iter()
            .map(|b| contrast_ratio(pack(c), pack(*b)))
            .fold(f32::INFINITY, f32::min)
    };
    if worst(color) >= minimum {
        return color;
    }
    let mut best = color;
    let mut best_contrast = worst(color);
    for target in preferred.into_iter().chain([[0; 3], [255; 3]]) {
        for step in 1..=100u8 {
            let candidate = mix(color, target, f32::from(step) / 100.0);
            let contrast = worst(candidate);
            if contrast > best_contrast {
                best = candidate;
                best_contrast = contrast;
            }
            if contrast >= minimum {
                return candidate;
            }
        }
    }
    best
}

impl Palette {
    pub fn base(preset: ThemePreset, light: bool) -> Self {
        match (preset, light) {
            (ThemePreset::Kite, false) => Self {
                light: false,
                shell: unpack(0x0d0d0d),
                editor_bg: unpack(0x09090b),
                preview_bg: unpack(0x0e0e11),
                popup: unpack(0x17171a),
                text: unpack(0xe8e8ea),
                muted: unpack(0xa9a9ae),
                faint: unpack(0x85858a),
                accent: unpack(0x8b7cf6),
                on_accent: on_accent_for(unpack(0x8b7cf6)),
                code_bg: unpack(0x16161b),
                border_rgba: "rgba(255, 255, 255, 0.09)",
                hairline_rgba: "rgba(255, 255, 255, 0.06)",
                row_hover_rgba: "rgba(235, 235, 235, 0.06)".into(),
                row_active_rgba: "rgba(235, 235, 235, 0.12)".into(),
                tint: None,
            },
            (ThemePreset::Kite, true) => Self {
                light: true,
                shell: unpack(0xf3f3f5),
                editor_bg: unpack(0xfafafb),
                preview_bg: unpack(0xffffff),
                popup: unpack(0xffffff),
                text: unpack(0x28282d),
                muted: unpack(0x62626a),
                faint: unpack(0x797981),
                accent: unpack(0x5b43e8),
                on_accent: on_accent_for(unpack(0x5b43e8)),
                code_bg: unpack(0xf1f1f5),
                border_rgba: "rgba(0, 0, 0, 0.11)",
                hairline_rgba: "rgba(0, 0, 0, 0.07)",
                row_hover_rgba: "rgba(26, 26, 26, 0.05)".into(),
                row_active_rgba: "rgba(26, 26, 26, 0.10)".into(),
                tint: None,
            },
            (ThemePreset::Github, true) => Self {
                light: true,
                shell: unpack(0xf6f8fa),
                editor_bg: unpack(0xfafbfc),
                preview_bg: unpack(0xffffff),
                popup: unpack(0xffffff),
                text: unpack(0x1f2328),
                muted: unpack(0x59636e),
                faint: unpack(0x6e7781),
                accent: unpack(0x0969da),
                on_accent: [255, 255, 255],
                code_bg: unpack(0xf3f4f6),
                border_rgba: "rgba(31, 35, 40, 0.12)",
                hairline_rgba: "rgba(31, 35, 40, 0.07)",
                row_hover_rgba: "rgba(31, 35, 40, 0.05)".into(),
                row_active_rgba: "rgba(9, 105, 218, 0.11)".into(),
                tint: None,
            },
            (ThemePreset::Github, false) => Self {
                light: false,
                shell: unpack(0x0d1117),
                editor_bg: unpack(0x0b0f14),
                preview_bg: unpack(0x10151c),
                popup: unpack(0x161b22),
                text: unpack(0xe6edf3),
                muted: unpack(0x9198a1),
                faint: unpack(0x7d8590),
                accent: unpack(0x58a6ff),
                on_accent: [15, 15, 18],
                code_bg: unpack(0x161b22),
                border_rgba: "rgba(240, 246, 252, 0.10)",
                hairline_rgba: "rgba(240, 246, 252, 0.06)",
                row_hover_rgba: "rgba(240, 246, 252, 0.06)".into(),
                row_active_rgba: "rgba(88, 166, 255, 0.14)".into(),
                tint: None,
            },
            (ThemePreset::Nord, false) => Self {
                light: false,
                shell: unpack(0x242933),
                editor_bg: unpack(0x20252e),
                preview_bg: unpack(0x262c36),
                popup: unpack(0x2e3440),
                text: unpack(0xeceff4),
                muted: unpack(0xb8c0cc),
                faint: unpack(0x8e98a8),
                accent: unpack(0x88c0d0),
                on_accent: [20, 24, 30],
                code_bg: unpack(0x2e3440),
                border_rgba: "rgba(236, 239, 244, 0.10)",
                hairline_rgba: "rgba(236, 239, 244, 0.06)",
                row_hover_rgba: "rgba(236, 239, 244, 0.06)".into(),
                row_active_rgba: "rgba(136, 192, 208, 0.16)".into(),
                tint: None,
            },
            (ThemePreset::Nord, true) => Self {
                light: true,
                shell: unpack(0xeceff4),
                editor_bg: unpack(0xf4f6f9),
                preview_bg: unpack(0xfafbfd),
                popup: unpack(0xffffff),
                text: unpack(0x2e3440),
                muted: unpack(0x4c566a),
                faint: unpack(0x6b768c),
                accent: unpack(0x5e81ac),
                on_accent: [255, 255, 255],
                code_bg: unpack(0xe5e9f0),
                border_rgba: "rgba(46, 52, 64, 0.12)",
                hairline_rgba: "rgba(46, 52, 64, 0.07)",
                row_hover_rgba: "rgba(46, 52, 64, 0.05)".into(),
                row_active_rgba: "rgba(94, 129, 172, 0.14)".into(),
                tint: None,
            },
            (ThemePreset::Sepia, true) => Self {
                light: true,
                shell: unpack(0xf3ece0),
                editor_bg: unpack(0xf8f3e9),
                preview_bg: unpack(0xfcf8f0),
                popup: unpack(0xfffcf6),
                text: unpack(0x3a2f24),
                muted: unpack(0x6d5d4d),
                faint: unpack(0x887866),
                accent: unpack(0x9f4f28),
                on_accent: [255, 255, 255],
                code_bg: unpack(0xefe6d8),
                border_rgba: "rgba(58, 47, 36, 0.12)",
                hairline_rgba: "rgba(58, 47, 36, 0.07)",
                row_hover_rgba: "rgba(58, 47, 36, 0.05)".into(),
                row_active_rgba: "rgba(159, 79, 40, 0.12)".into(),
                tint: None,
            },
            (ThemePreset::Sepia, false) => Self {
                light: false,
                shell: unpack(0x1b1714),
                editor_bg: unpack(0x171411),
                preview_bg: unpack(0x1e1a16),
                popup: unpack(0x26211c),
                text: unpack(0xebe2d5),
                muted: unpack(0xb7aa99),
                faint: unpack(0x908374),
                accent: unpack(0xd99b52),
                on_accent: [22, 18, 14],
                code_bg: unpack(0x26211c),
                border_rgba: "rgba(235, 226, 213, 0.10)",
                hairline_rgba: "rgba(235, 226, 213, 0.06)",
                row_hover_rgba: "rgba(235, 226, 213, 0.06)".into(),
                row_active_rgba: "rgba(217, 155, 82, 0.15)".into(),
                tint: None,
            },
        }
    }

    pub fn tinted(mut self, color: [u8; 3]) -> Self {
        let tint = premixed(color, self.light);
        let lean = |c: [u8; 3]| mix(c, tint, SURFACE_MIX);
        self.shell = lean(self.shell);
        self.editor_bg = lean(self.editor_bg);
        self.preview_bg = lean(self.preview_bg);
        self.popup = lean(self.popup);
        self.code_bg = lean(self.code_bg);
        self.tint = Some(color);

        let surfaces = [self.shell, self.editor_bg, self.preview_bg, self.popup];
        self.text = harden(self.text, &surfaces, TEXT_CONTRAST, None);
        self.muted = harden(self.muted, &surfaces, TEXT_CONTRAST, Some(self.text));
        self.faint = harden(self.faint, &surfaces, FAINT_CONTRAST, Some(self.text));

        self.accent = accent_for(color, self.preview_bg);
        self.on_accent = on_accent_for(self.accent);
        if self.light {
            self.row_hover_rgba = rgba_str(self.accent, 0.09);
            self.row_active_rgba = rgba_str(self.accent, 0.15);
        } else {
            self.row_hover_rgba = "rgba(255, 255, 255, 0.09)".into();
            self.row_active_rgba = "rgba(255, 255, 255, 0.15)".into();
        }
        self
    }

    pub fn resolve(
        self,
        backdrop: Option<&[[u8; 3]]>,
        fills_window: bool,
        hero_opacity: f32,
    ) -> ResolvedTheme {
        let panel_alpha = if fills_window {
            if let Some(bd) = backdrop {
                let words = [
                    (self.text, TEXT_CONTRAST),
                    (self.muted, TEXT_CONTRAST),
                    (self.faint, FAINT_CONTRAST),
                ];
                let base_alpha = panel::panel_alpha(
                    bd,
                    &[
                        Panel {
                            surface: self.shell,
                            texts: &words,
                        },
                        Panel {
                            surface: self.editor_bg,
                            texts: &words,
                        },
                        Panel {
                            surface: self.preview_bg,
                            texts: &words,
                        },
                    ],
                );
                // When user lowers Visibility below 100%, gently blend panel toward opaque so
                // visibility directly controls how strongly the wallpaper shows through!
                let vis = hero_opacity.clamp(0.1, 1.0);
                (1.0 - (1.0 - base_alpha) * vis).clamp(panel::MIN_ALPHA, 0.98)
            } else {
                1.0
            }
        } else {
            1.0
        };

        ResolvedTheme {
            light: self.light,
            shell: hex_str(self.shell),
            shell_panel: rgba_str(self.shell, panel_alpha),
            editor_bg: hex_str(self.editor_bg),
            editor_panel: rgba_str(self.editor_bg, (panel_alpha + 0.03).min(0.98)),
            preview_bg: hex_str(self.preview_bg),
            preview_panel: rgba_str(self.preview_bg, panel_alpha),
            popup: hex_str(self.popup),
            text: hex_str(self.text),
            muted: hex_str(self.muted),
            faint: hex_str(self.faint),
            accent: hex_str(self.accent),
            accent_soft: rgba_str(self.accent, if self.light { 0.14 } else { 0.20 }),
            on_accent: hex_str(self.on_accent),
            code_bg: rgba_str(self.code_bg, if panel_alpha < 0.99 { 0.82 } else { 1.0 }),
            border: self.border_rgba.to_string(),
            hairline: self.hairline_rgba.to_string(),
            row_hover: self.row_hover_rgba,
            row_active: self.row_active_rgba,
            panel_alpha,
        }
    }
}
