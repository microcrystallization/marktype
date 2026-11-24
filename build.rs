// Embed the Windows icon and version on both MSVC and GNU builds.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/marktype.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let assets_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
            .join("assets");
    let major = env!("CARGO_PKG_VERSION_MAJOR");
    let minor = env!("CARGO_PKG_VERSION_MINOR");
    let patch = env!("CARGO_PKG_VERSION_PATCH");
    let version = env!("CARGO_PKG_VERSION");
    std::fs::write(
        out_dir.join("marktype-version.h"),
        format!(
            "#define MARKTYPE_VERSION_NUMBERS {major},{minor},{patch},0\n\
             #define MARKTYPE_FILE_VERSION \"{major}.{minor}.{patch}.0\"\n\
             #define MARKTYPE_VERSION \"{version}\"\n"
        ),
    )
    .expect("Write Windows version resource header");
    embed_resource::compile_for(
        "packaging/windows/marktype.rc",
        ["marktype"],
        embed_resource::ParamsIncludeDirs([&out_dir, &assets_dir]),
    )
    .manifest_required()
    .expect("Compile MarkType's Windows icon and version resources");

    #[cfg(target_os = "windows")]
    {
        // GNU builds use the side-by-side WebView2 loader.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("gnu") {
            return;
        }

        let Some(build_dir) = out_dir.ancestors().find(|path| path.ends_with("build")) else {
            return;
        };
        let profile_dir = build_dir.parent().expect("Cargo profile directory");
        let loader_arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("x86_64") => "x64",
            Ok("x86") => "x86",
            Ok("aarch64") => "arm64",
            _ => return,
        };
        if let Ok(entries) = std::fs::read_dir(build_dir) {
            for entry in entries.flatten() {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("webview2-com-sys")
                {
                    continue;
                }
                if let Some(loader) = find_webview2_loader(&entry.path(), loader_arch) {
                    let _ = std::fs::copy(loader, profile_dir.join("WebView2Loader.dll"));
                    break;
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn find_webview2_loader(dir: &std::path::Path, arch: &str) -> Option<PathBuf> {
    let loader = dir.join("out").join(arch).join("WebView2Loader.dll");
    if loader.is_file() {
        return Some(loader);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| {
            entry
                .path()
                .join("out")
                .join(arch)
                .join("WebView2Loader.dll")
        })
        .find(|loader| loader.is_file())
}
