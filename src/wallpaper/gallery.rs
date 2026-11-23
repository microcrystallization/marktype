// Built-in classical wallpapers migrated from Kite (https://github.com/Kfkcome/kite),
// public-domain works from The Metropolitan Museum of Art Open Access (CC0 1.0).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

use crate::settings::Settings;

pub struct Builtin {
    pub id: &'static str,
    pub title: &'static str,
    pub bytes: &'static [u8],
}

macro_rules! builtin {
    ($id:literal, $title:literal) => {
        Builtin {
            id: $id,
            title: $title,
            bytes: include_bytes!(concat!("../../assets/wallpapers/", $id, ".jpg")),
        }
    };
}

pub static BUILTINS: [Builtin; 7] = [
    builtin!("great-wave", "The Great Wave — Hokusai"),
    builtin!("red-fuji", "Red Fuji — Hokusai"),
    builtin!("kanbara-snow", "Evening Snow at Kanbara — Hiroshige"),
    builtin!("shono-rain", "Sudden Shower at Shōno — Hiroshige"),
    builtin!("heart-of-the-andes", "Heart of the Andes — Church"),
    builtin!("wheat-field", "Wheat Field with Cypresses — van Gogh"),
    builtin!(
        "fontainebleau-morning",
        "Morning in the Forest of Fontainebleau — Rousseau"
    ),
];

pub const THUMB_W: u32 = 240;
pub const THUMB_H: u32 = 150;
const THUMB_VERSION: u32 = 1;

fn builtin_dir(config: &Path) -> PathBuf {
    super::store::store_dir(config).join("builtin")
}

pub fn builtin_path(config: &Path, b: &Builtin) -> PathBuf {
    builtin_dir(config).join(format!("{}.jpg", b.id))
}

pub fn find_builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

pub fn install_builtin(config: &Path, b: &Builtin) -> Result<PathBuf, String> {
    let target = builtin_path(config, b);
    if std::fs::metadata(&target).is_ok_and(|m| m.len() == b.bytes.len() as u64) {
        return Ok(target);
    }
    let dir = builtin_dir(config);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {dir:?}: {e}"))?;
    std::fs::write(&target, b.bytes).map_err(|e| format!("Cannot write the wallpaper: {e}"))?;
    Ok(target)
}

pub fn use_picked(st: &mut Settings, path: String) {
    super::store::remember(&mut st.wallpaper_history, &path);
    st.wallpaper = Some(path);
}

pub fn use_builtin(st: &mut Settings, path: String) {
    st.wallpaper = Some(path);
}

pub fn thumb_name(id: &str, len: u64) -> String {
    let mut hasher = DefaultHasher::new();
    (id, len, THUMB_W, THUMB_H, THUMB_VERSION).hash(&mut hasher);
    format!("{:016x}.jpg", hasher.finish())
}

pub fn thumb_dir(config: &Path) -> PathBuf {
    super::store::store_dir(config).join("thumbs")
}

pub fn thumbnail(
    config: &Path,
    id: &str,
    len: u64,
    load: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let path = thumb_dir(config).join(thumb_name(id, len));
    if path.is_file() {
        return Ok(path);
    }
    let bytes = load()?;
    let full =
        image::load_from_memory(&bytes).map_err(|e| format!("Cannot decode wallpaper: {e}"))?;
    let thumb = DynamicImage::ImageRgb8(
        full.resize_to_fill(THUMB_W, THUMB_H, FilterType::Triangle)
            .to_rgb8(),
    );
    let dir = thumb_dir(config);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {dir:?}: {e}"))?;
    thumb
        .save_with_format(&path, ImageFormat::Jpeg)
        .map_err(|e| format!("Cannot save thumbnail: {e}"))?;
    Ok(path)
}

pub fn builtin_thumbnail(config: &Path, b: &Builtin) -> Result<PathBuf, String> {
    thumbnail(
        config,
        &format!("builtin:{}", b.id),
        b.bytes.len() as u64,
        || Ok(b.bytes.to_vec()),
    )
}

pub fn file_thumbnail(config: &Path, source: &Path) -> Result<PathBuf, String> {
    let len = std::fs::metadata(source)
        .map_err(|e| format!("Cannot read image: {e}"))?
        .len();
    thumbnail(config, &source.to_string_lossy(), len, || {
        std::fs::read(source).map_err(|e| format!("Cannot read image: {e}"))
    })
}
