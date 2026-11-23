// Automatically compiles Windows icon/version resource into marktype.exe and copies
// x64 WebView2Loader.dll next to the built executable on Windows GNU targets.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/marktype.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(target_os = "windows")]
    {
        let out_dir = match std::env::var_os("OUT_DIR") {
            Some(d) => PathBuf::from(d),
            None => return,
        };

        // 1. Embed Windows application icon & version metadata into marktype.exe
        let rc_path = PathBuf::from("packaging/windows/marktype.rc");
        if rc_path.is_file() {
            let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
            if target_env == "gnu" {
                let res_obj = out_dir.join("marktype_res.o");
                if let Ok(status) = Command::new("windres")
                    .args([
                        rc_path.to_str().unwrap_or("packaging/windows/marktype.rc"),
                        "-O",
                        "coff",
                        "-o",
                        res_obj.to_str().unwrap_or("marktype_res.o"),
                    ])
                    .status()
                    && status.success()
                {
                    println!("cargo:rustc-link-arg-bin=marktype={}", res_obj.display());
                }
            }
        }

        // 2. Copy x64 WebView2Loader.dll next to target/<profile>/marktype.exe
        let Some(profile_dir) = out_dir.ancestors().nth(3) else {
            return;
        };
        let build_dir = profile_dir.join("build");
        if let Ok(entries) = std::fs::read_dir(&build_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("webview2-com-sys"))
                    && let Some(found) = find_webview2_dll(&p)
                {
                    let dest = profile_dir.join("WebView2Loader.dll");
                    let _ = std::fs::copy(found, dest);
                    break;
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn find_webview2_dll(dir: &std::path::Path) -> Option<PathBuf> {
    let direct = dir.join("out").join("x64").join("WebView2Loader.dll");
    if direct.is_file() {
        return Some(direct);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir()
                && let Some(found) = find_webview2_dll(&p)
            {
                return Some(found);
            }
        }
    }
    None
}
