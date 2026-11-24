use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::wallpaper::fx::Effect;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    #[default]
    Split,
    Live,
    Source,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreset {
    #[default]
    Kite,
    Github,
    Nord,
    Sepia,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub size: [f64; 2],
    pub position: Option<[i32; 2]>,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1420.0, 900.0],
            position: None,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub language: String,
    pub light: bool,
    pub theme_preset: ThemePreset,
    pub view_mode: ViewMode,
    pub font_size: f32,
    pub preview_max_width: u32,
    pub line_height: f32,
    pub sync_scroll: bool,
    pub focus_mode: bool,
    pub typewriter_mode: bool,
    pub auto_save: bool,
    pub show_line_numbers: bool,
    pub reduce_motion: bool,
    pub sidebar_open: bool,
    pub sidebar_tab: String,
    pub split_ratio: f32,
    // Kite-style wallpaper settings
    pub wallpaper: Option<String>,
    pub wallpaper_history: Vec<String>,
    pub wallpaper_effect: Effect,
    pub wallpaper_hero_opacity: f32,
    pub wallpaper_fills_window: bool,
    pub wallpaper_theme_colors: bool,
    // Recent documents & workspace
    pub recent_files: Vec<String>,
    pub last_folder: Option<String>,
    pub window_geometry: WindowGeometry,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
            light: false,
            theme_preset: ThemePreset::Kite,
            view_mode: ViewMode::Split,
            font_size: 14.0,
            preview_max_width: 860,
            line_height: 1.72,
            sync_scroll: true,
            focus_mode: false,
            typewriter_mode: false,
            auto_save: false,
            show_line_numbers: true,
            reduce_motion: false,
            sidebar_open: true,
            sidebar_tab: "outline".to_string(),
            split_ratio: 0.48,
            wallpaper: None,
            wallpaper_history: Vec::new(),
            wallpaper_effect: Effect::None,
            wallpaper_hero_opacity: 1.0,
            wallpaper_fills_window: true,
            wallpaper_theme_colors: true,
            recent_files: Vec::new(),
            last_folder: None,
            window_geometry: WindowGeometry::default(),
        }
    }
}

impl Settings {
    pub fn clamp(&mut self) {
        self.window_geometry.size[0] = self.window_geometry.size[0].clamp(820.0, 10000.0);
        self.window_geometry.size[1] = self.window_geometry.size[1].clamp(560.0, 10000.0);
        self.font_size = self.font_size.clamp(10.0, 28.0);
        self.preview_max_width = self.preview_max_width.clamp(560, 1400);
        self.line_height = self.line_height.clamp(1.3, 2.2);
        self.wallpaper_hero_opacity = (self.wallpaper_hero_opacity * 10.0).round() / 10.0;
        self.wallpaper_hero_opacity = self.wallpaper_hero_opacity.clamp(0.1, 1.0);
        self.split_ratio = self.split_ratio.clamp(0.2, 0.8);
    }

    pub fn remember_recent_file(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_owned());
        self.recent_files.truncate(12);
    }
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("MARKTYPE_CONFIG_DIR").filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    directories::ProjectDirs::from("com", "marktype", "MarkType")
        .map(|d| d.config_dir().to_path_buf())
}

fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        let mut s = Settings::default();
        // Default first-run experience: activate Great Wave built-in wallpaper so the app looks extraordinary immediately!
        if let Some(dir) = config_dir()
            && let Ok(wp) = crate::wallpaper::gallery::install_builtin(
                &dir,
                &crate::wallpaper::gallery::BUILTINS[0],
            )
        {
            s.wallpaper = Some(wp.to_string_lossy().into_owned());
        }
        let _ = save(&s);
        return s;
    };
    let mut s: Settings = serde_json::from_slice(&bytes).unwrap_or_default();
    s.clamp();
    s
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let Some(path) = settings_path() else {
        return Err("Cannot resolve config directory".into());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create config dir: {e}"))?;
    }
    crate::storage::atomic_json(&path, settings).map_err(|e| format!("Cannot save settings: {e}"))
}
