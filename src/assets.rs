//! Embedded static assets (UI, KaTeX + 20 math fonts, Geist fonts, and 7 Kite wallpapers)
//! plus custom protocol router for `http://marktype.localhost/...`.

use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use regex::{Captures, Regex};

use crate::settings;
use crate::wallpaper::gallery;
use crate::wallpaper::store;

const INDEX_HTML: &[u8] = include_bytes!("../ui/index.html");
const ICON_SVG: &[u8] = include_bytes!("../assets/icon.svg");
const STYLES_CSS: &[u8] = include_bytes!("../ui/styles.css");
const SYNTAX_CSS: &[u8] = include_bytes!("../ui/syntax.css");
const I18N_JS: &[u8] = include_bytes!("../ui/i18n.js");
const SOURCE_JS: &[u8] = include_bytes!("../ui/source.js");
const APP_JS: &[u8] = include_bytes!("../ui/app.js");

const KATEX_JS: &[u8] = include_bytes!("../assets/katex/katex.min.js");
const KATEX_CSS: &[u8] = include_bytes!("../assets/katex/katex.min.css");
const MHCHEM_JS: &[u8] = include_bytes!("../assets/katex/mhchem.min.js");
pub const APP_URL: &str = "http://marktype.localhost/";
pub const APP_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data: https: http:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'";

#[derive(Default)]
struct LocalImages {
    paths: Vec<PathBuf>,
    by_path: std::collections::HashMap<PathBuf, usize>,
    picked: std::collections::HashMap<PathBuf, PathBuf>,
}

static LOCAL_IMAGES: LazyLock<Mutex<LocalImages>> = LazyLock::new(Mutex::default);
const MAX_LOCAL_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

pub fn register_picked_image(path: &Path) {
    if let Ok(canonical) = dunce::canonicalize(path)
        && mime_from_path(&canonical).starts_with("image/")
    {
        LOCAL_IMAGES
            .lock()
            .unwrap()
            .picked
            .insert(path.to_path_buf(), canonical);
    }
}

pub(crate) fn image_url(dest: &str, doc_dir: Option<&Path>) -> Option<String> {
    let trimmed = dest.trim();
    let lower = trimmed.to_ascii_lowercase();
    if ["http://", "https://", "data:image/"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return Some(trimmed.to_owned());
    }
    let decoded = percent_decode_str(trimmed).decode_utf8().ok()?;
    let relative = Path::new(decoded.as_ref());
    let path = if relative.is_absolute() {
        let picked = LOCAL_IMAGES.lock().unwrap().picked.get(relative).cloned();
        if let Some(path) = picked {
            path
        } else {
            let directory = doc_dir?;
            // Check the path before accessing it, then resolve symlinks and check again.
            let relative_path = relative.strip_prefix(directory).ok()?;
            local_path_in(directory, relative_path)?
        }
    } else {
        local_path_in(doc_dir?, relative)?
    };
    if !mime_from_path(&path).starts_with("image/") {
        return None;
    }
    let mut images = LOCAL_IMAGES.lock().unwrap();
    let id = if let Some(&id) = images.by_path.get(&path) {
        id
    } else {
        let id = images.paths.len();
        images.paths.push(path.clone());
        images.by_path.insert(path, id);
        id
    };
    Some(format!("{APP_URL}local-image/{id}"))
}

fn local_path_in(directory: &Path, relative: &Path) -> Option<PathBuf> {
    if !relative.components().all(|component| match component {
        Component::Normal(part) => !part.to_string_lossy().contains(':'),
        Component::CurDir => true,
        _ => false,
    }) {
        return None;
    }
    let directory = dunce::canonicalize(directory).ok()?;
    let path = dunce::canonicalize(directory.join(relative)).ok()?;
    path.starts_with(directory).then_some(path)
}

fn local_image(id: usize) -> Option<(&'static str, Vec<u8>)> {
    use std::io::Read;
    let path = LOCAL_IMAGES.lock().ok()?.paths.get(id)?.clone();
    if dunce::canonicalize(&path).ok()? != path {
        return None;
    }
    let file = std::fs::File::open(&path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LOCAL_IMAGE_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_LOCAL_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_LOCAL_IMAGE_BYTES).then(|| (mime_from_path(&path), bytes))
}

fn katex_font(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "KaTeX_AMS-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_AMS-Regular.woff2")
        }
        "KaTeX_Caligraphic-Bold.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Caligraphic-Bold.woff2")
        }
        "KaTeX_Caligraphic-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Caligraphic-Regular.woff2")
        }
        "KaTeX_Fraktur-Bold.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Fraktur-Bold.woff2")
        }
        "KaTeX_Fraktur-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Fraktur-Regular.woff2")
        }
        "KaTeX_Main-Bold.woff2" => include_bytes!("../assets/katex/fonts/KaTeX_Main-Bold.woff2"),
        "KaTeX_Main-BoldItalic.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Main-BoldItalic.woff2")
        }
        "KaTeX_Main-Italic.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Main-Italic.woff2")
        }
        "KaTeX_Main-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Main-Regular.woff2")
        }
        "KaTeX_Math-BoldItalic.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Math-BoldItalic.woff2")
        }
        "KaTeX_Math-Italic.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Math-Italic.woff2")
        }
        "KaTeX_SansSerif-Bold.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Bold.woff2")
        }
        "KaTeX_SansSerif-Italic.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Italic.woff2")
        }
        "KaTeX_SansSerif-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_SansSerif-Regular.woff2")
        }
        "KaTeX_Script-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Script-Regular.woff2")
        }
        "KaTeX_Size1-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Size1-Regular.woff2")
        }
        "KaTeX_Size2-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Size2-Regular.woff2")
        }
        "KaTeX_Size3-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Size3-Regular.woff2")
        }
        "KaTeX_Size4-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Size4-Regular.woff2")
        }
        "KaTeX_Typewriter-Regular.woff2" => {
            include_bytes!("../assets/katex/fonts/KaTeX_Typewriter-Regular.woff2")
        }
        _ => return None,
    })
}

fn app_font(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "Geist.ttf" => include_bytes!("../assets/fonts/Geist.ttf"),
        "Geist-Medium.ttf" => include_bytes!("../assets/fonts/Geist-Medium.ttf"),
        "Geist-SemiBold.ttf" => include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
        "GeistMono.ttf" => include_bytes!("../assets/fonts/GeistMono.ttf"),
        "GeistMono-Bold.ttf" => include_bytes!("../assets/fonts/GeistMono-Bold.ttf"),
        "GeistMono-Italic.ttf" => include_bytes!("../assets/fonts/GeistMono-Italic.ttf"),
        "GeistMono-BoldItalic.ttf" => include_bytes!("../assets/fonts/GeistMono-BoldItalic.ttf"),
        _ => return None,
    })
}

pub fn serve_request(uri: &str) -> (u16, &'static str, Vec<u8>) {
    // Strip scheme + host (`http://marktype.localhost` or `marktype://localhost`)
    let path_and_query = if let Some(idx) = uri.find("://") {
        let rest = &uri[idx + 3..];
        match rest.find('/') {
            Some(slash) => &rest[slash..],
            None => "/",
        }
    } else {
        uri
    };

    let path = path_and_query.split('?').next().unwrap_or(path_and_query);

    match path {
        "/" | "/index.html" => (200, "text/html; charset=utf-8", INDEX_HTML.to_vec()),
        "/icon.svg" => (200, "image/svg+xml", ICON_SVG.to_vec()),
        "/styles.css" => (200, "text/css; charset=utf-8", STYLES_CSS.to_vec()),
        "/syntax.css" => (200, "text/css; charset=utf-8", SYNTAX_CSS.to_vec()),
        "/i18n.js" => (
            200,
            "application/javascript; charset=utf-8",
            I18N_JS.to_vec(),
        ),
        "/app.js" => (
            200,
            "application/javascript; charset=utf-8",
            APP_JS.to_vec(),
        ),
        "/source.js" => (
            200,
            "application/javascript; charset=utf-8",
            SOURCE_JS.to_vec(),
        ),
        "/katex/katex.min.js" => (
            200,
            "application/javascript; charset=utf-8",
            KATEX_JS.to_vec(),
        ),
        "/katex/katex.min.css" => (200, "text/css; charset=utf-8", KATEX_CSS.to_vec()),
        "/katex/mhchem.min.js" => (
            200,
            "application/javascript; charset=utf-8",
            MHCHEM_JS.to_vec(),
        ),
        _ if path.starts_with("/katex/fonts/") => {
            let name = &path["/katex/fonts/".len()..];
            if let Some(bytes) = katex_font(name) {
                (200, "font/woff2", bytes.to_vec())
            } else {
                (404, "text/plain", b"Font not found".to_vec())
            }
        }
        _ if path.starts_with("/fonts/") => {
            let name = &path["/fonts/".len()..];
            if let Some(bytes) = app_font(name) {
                (200, "font/ttf", bytes.to_vec())
            } else {
                (404, "text/plain", b"Font not found".to_vec())
            }
        }
        _ if path.starts_with("/wallpaper/builtin/") => {
            let file = &path["/wallpaper/builtin/".len()..];
            let id = file.trim_end_matches(".jpg");
            if let Some(b) = gallery::find_builtin(id) {
                (200, "image/jpeg", b.bytes.to_vec())
            } else {
                (404, "text/plain", b"Builtin wallpaper not found".to_vec())
            }
        }
        _ if path.starts_with("/wallpaper/cache/") => {
            let file = &path["/wallpaper/cache/".len()..];
            if is_safe_filename(file)
                && let Some(cfg) = settings::config_dir()
            {
                let target = store::cache_dir(&cfg).join(file);
                if let Ok(bytes) = std::fs::read(&target) {
                    return (200, mime_from_path(&target), bytes);
                }
            }
            (404, "text/plain", b"Cached wallpaper not found".to_vec())
        }
        _ if path.starts_with("/wallpaper/thumb/") => {
            let file = &path["/wallpaper/thumb/".len()..];
            if is_safe_filename(file)
                && let Some(cfg) = settings::config_dir()
            {
                let target = gallery::thumb_dir(&cfg).join(file);
                if let Ok(bytes) = std::fs::read(&target) {
                    return (200, "image/jpeg", bytes);
                }
            }
            (404, "text/plain", b"Thumbnail not found".to_vec())
        }
        _ if path.starts_with("/local-image/") => path["/local-image/".len()..]
            .parse::<usize>()
            .ok()
            .and_then(local_image)
            .map(|(mime, bytes)| (200, mime, bytes))
            .unwrap_or_else(|| (404, "text/plain", b"Local image not found".to_vec())),
        _ => (404, "text/plain", b"Not found".to_vec()),
    }
}

fn is_safe_filename(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.contains("..")
}

fn mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

pub fn export_html_template(title: &str, body_html: &str, light: bool) -> String {
    static KATEX_INLINE_CSS: LazyLock<String> = LazyLock::new(|| {
        let css = std::str::from_utf8(KATEX_CSS).unwrap_or("");
        let fonts = Regex::new(r#"url\(fonts/([^)]*\.woff2)\) format\("woff2"\),url\(fonts/[^)]*\) format\("woff"\),url\(fonts/[^)]*\) format\("truetype"\)"#).unwrap();
        fonts
            .replace_all(css, |cap: &Captures<'_>| match katex_font(&cap[1]) {
                Some(bytes) => format!(
                    "url(data:font/woff2;base64,{}) format(\"woff2\")",
                    STANDARD.encode(bytes)
                ),
                None => cap[0].to_owned(),
            })
            .into_owned()
    });
    static LOCAL_SRC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"src="http://marktype\.localhost/local-image/(\d+)""#).unwrap()
    });
    let body_html = LOCAL_SRC.replace_all(body_html, |cap: &Captures<'_>| {
        if let Some((mime, bytes)) = cap[1].parse().ok().and_then(local_image) {
            format!("src=\"data:{mime};base64,{}\"", STANDARD.encode(bytes))
        } else {
            "src=\"\"".into()
        }
    });
    let katex_css = &*KATEX_INLINE_CSS;
    let katex_js = std::str::from_utf8(KATEX_JS).unwrap_or("");
    let mhchem_js = std::str::from_utf8(MHCHEM_JS).unwrap_or("");
    let syntax = std::str::from_utf8(SYNTAX_CSS).unwrap_or("");
    let theme = if light { "light" } else { "dark" };
    let bg = if light { "#ffffff" } else { "#0e0e11" };
    let fg = if light { "#24292f" } else { "#e8e8ea" };
    let code_bg = if light { "#f4f5f7" } else { "#18181f" };
    let border = if light { "#e2e5ea" } else { "#2c2c35" };
    let accent = if light { "#5b43e8" } else { "#8b7cf6" };
    let safe_title = crate::highlight::escape_html_string(title);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{safe_title}</title>
<style>{katex_css}</style>
<style>
  {syntax}
  body {{
    margin: 0; padding: 48px 24px;
    background: {bg}; color: {fg};
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
    line-height: 1.75; font-size: 16px;
  }}
  main {{ max-width: 820px; margin: 0 auto; }}
  h1, h2, h3, h4 {{ font-weight: 650; line-height: 1.3; margin-top: 1.6em; }}
  h1 {{ font-size: 2.1em; border-bottom: 1px solid {border}; padding-bottom: 0.3em; }}
  h2 {{ font-size: 1.55em; border-bottom: 1px solid {border}; padding-bottom: 0.25em; }}
  a {{ color: {accent}; }}
  code.mt-inline-code {{ background: {code_bg}; padding: 2px 6px; border-radius: 5px; font-family: Consolas, monospace; font-size: 0.9em; }}
  .mt-code-wrap {{ background: {code_bg}; border: 1px solid {border}; border-radius: 10px; overflow: hidden; margin: 1.2em 0; }}
  .mt-code-head {{ padding: 6px 12px; font-size: 12px; opacity: 0.7; border-bottom: 1px solid {border}; display: flex; justify-content: space-between; }}
  .mt-code-copy {{ display: none; }}
  pre.mt-code-pre {{ margin: 0; padding: 14px 16px; overflow-x: auto; font-family: Consolas, monospace; font-size: 13.5px; }}
  blockquote {{ border-left: 3px solid {accent}; margin: 1em 0; padding: 4px 16px; opacity: 0.9; }}
  table {{ border-collapse: collapse; width: 100%; margin: 1.2em 0; }}
  th, td {{ border: 1px solid {border}; padding: 8px 12px; }}
  img {{ max-width: 100%; border-radius: 8px; }}
  .mt-math-block {{ margin: 1.2em 0; text-align: center; overflow-x: auto; }}
  .mt-callout {{ border-left: 4px solid {accent}; background: {code_bg}; border-radius: 8px; padding: 12px 16px; margin: 1.2em 0; }}
  .mt-callout-title {{ font-weight: 600; margin-bottom: 6px; }}
  mark {{ background: rgba(250, 204, 21, 0.35); color: inherit; padding: 1px 4px; border-radius: 3px; }}
</style>
</head>
<body class="{theme}">
<main>{body_html}</main>
<script>{katex_js}</script>
<script>{mhchem_js}</script>
<script>
document.querySelectorAll('.mt-math-block').forEach(el => {{
  try {{ katex.render(el.dataset.tex || '', el, {{ displayMode: true, throwOnError: false }}); }} catch {{}}
}});
document.querySelectorAll('.mt-math-inline').forEach(el => {{
  try {{ katex.render(el.dataset.tex || '', el, {{ displayMode: false, throwOnError: false }}); }} catch {{}}
}});
</script>
</body>
</html>"#
    )
}
