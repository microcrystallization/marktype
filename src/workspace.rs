use std::io::{self, BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub fn document_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "mdown" | "mkd" | "txt")
    ) || crate::latex::project_source(path)
}

pub(crate) fn existing_inside(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let path = dunce::canonicalize(path)
        .map_err(|error| format!("Cannot locate workspace item: {error}"))?;
    if !path.starts_with(root) {
        return Err("The item is outside the workspace".into());
    }
    Ok(path)
}

pub(crate) fn child_path(dir: &Path, name: &str, markdown: bool) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.contains(['/', '\\', ':'])
        || !matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
    {
        return Err("Enter a single file or folder name".into());
    }
    let path = dir.join(name);
    if markdown && !document_file(&path) {
        return Ok(dir.join(format!("{name}.md")));
    }
    Ok(path)
}

pub(crate) fn rename_item(path: &Path, target: &Path) -> Result<(), String> {
    let rename_error = |error| format!("Cannot rename workspace item: {error}");
    match std::fs::symlink_metadata(target) {
        Ok(_) => {
            let name = target.file_name().expect("workspace item name");
            // On a case-insensitive volume the lookup can resolve to the old spelling.
            let case_change = path.file_name().is_some_and(|old| {
                old.to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            });
            if !case_change {
                return Err("An item already has that name".into());
            }
            let parent = path.parent().expect("workspace item parent");
            for entry in std::fs::read_dir(parent).map_err(rename_error)? {
                if entry.map_err(rename_error)?.file_name() == name {
                    return Err("An item already has that name".into());
                }
            }
            // This is a spelling change for the existing entry on a case-insensitive volume.
            return std::fs::rename(path, target).map_err(rename_error);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(rename_error(error)),
    }
    rename_without_replace(path, target).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            "An item already has that name".into()
        } else {
            rename_error(error)
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn rename_without_replace(path: &Path, target: &Path) -> io::Result<()> {
    use rustix::fs::{self, CWD, RenameFlags};
    fs::renameat_with(CWD, path, CWD, target, RenameFlags::NOREPLACE).map_err(Into::into)
}

#[cfg(target_os = "windows")]
fn rename_without_replace(path: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let encode = |name: &Path| {
        name.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let parent = std::fs::canonicalize(path.parent().expect("workspace item parent"))?;
    let path = encode(&parent.join(path.file_name().expect("workspace item name")));
    let target = encode(&parent.join(target.file_name().expect("target item name")));
    // Both buffers are UTF-16 strings with trailing NULs and live through the call.
    if unsafe { MoveFileExW(path.as_ptr(), target.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn rename_without_replace(path: &Path, target: &Path) -> io::Result<()> {
    std::fs::rename(path, target)
}

pub(crate) fn create_document(path: &Path) -> std::io::Result<()> {
    let heading = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    writeln!(file, "# {heading}\n")
}

pub(crate) fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ());
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ());
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let result = open::that_detached(if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    });
    result.map_err(|error| format!("Cannot show workspace item: {error}"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub request: u64,
    pub query: String,
    pub case_sensitive: bool,
    pub use_regex: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Match {
    path: PathBuf,
    relative: PathBuf,
    line: usize,
    col: usize,
    end_col: usize,
    excerpt: String,
}

pub fn search(root: &Path, request: SearchRequest, latest: &AtomicU64) -> Value {
    const LIMIT: usize = 500;
    let pattern = if request.use_regex {
        request.query
    } else {
        regex::escape(&request.query)
    };
    let matcher = match regex::RegexBuilder::new(&pattern)
        .case_insensitive(!request.case_sensitive)
        .build()
    {
        Ok(matcher) => matcher,
        Err(error) => {
            return json!({"event":"workspaceSearchResults","request":request.request,"error":error.to_string()});
        }
    };
    let mut matches = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .follow_links(false)
        .sort_by_file_path(Path::cmp)
        .build();
    'files: for entry in walker.filter_map(Result::ok) {
        if latest.load(Ordering::Relaxed) != request.request {
            break;
        }
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !document_file(path) {
            continue;
        }
        let Ok(file) = std::fs::File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut raw = Vec::new();
        let mut index = 0;
        loop {
            if latest.load(Ordering::Relaxed) != request.request {
                break 'files;
            }
            raw.clear();
            match reader.read_until(b'\n', &mut raw) {
                Ok(n) if n > 0 => {}
                _ => break,
            }
            if let Ok(line) = std::str::from_utf8(&raw)
                && let Some(found) = matcher.find(line.trim_end_matches(['\r', '\n']))
            {
                let start = line[..found.start()]
                    .char_indices()
                    .rev()
                    .nth(48)
                    .map_or(0, |(i, _)| i);
                let excerpt = line[start..].trim_end().chars().take(180).collect();
                let col = line[..found.start()].chars().count();
                matches.push(Match {
                    path: path.to_path_buf(),
                    relative: path.strip_prefix(root).unwrap_or(path).to_path_buf(),
                    line: index,
                    col,
                    end_col: col + found.as_str().chars().count(),
                    excerpt,
                });
                if matches.len() > LIMIT {
                    break 'files;
                }
            }
            index += 1;
        }
    }
    let truncated = matches.len() > LIMIT;
    matches.truncate(LIMIT);
    json!({"event":"workspaceSearchResults","request":request.request,"matches":matches,"truncated":truncated})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_search_honors_ignore_rules_and_returns_unicode_columns() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.md\n").unwrap();
        std::fs::write(root.join("ignored.md"), "hello Needle").unwrap();
        std::fs::write(root.join("notes.md"), "first\n🌊 needle here\nlast").unwrap();
        std::fs::write(root.join("binary.bin"), "needle").unwrap();
        let results = search(
            root,
            SearchRequest {
                request: 7,
                query: "NEEDLE".into(),
                case_sensitive: false,
                use_regex: false,
            },
            &AtomicU64::new(7),
        );
        let matches = results["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["relative"], "notes.md");
        assert_eq!(matches[0]["line"], 1);
        assert_eq!(matches[0]["col"], 2);
        assert_eq!(matches[0]["endCol"], 8);
    }

    #[test]
    fn workspace_creation_does_not_replace_an_existing_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = child_path(dir.path(), "notes", true).unwrap();
        create_document(&path).unwrap();
        std::fs::write(&path, "Keep this text").unwrap();
        assert!(create_document(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Keep this text");
        assert!(child_path(dir.path(), "../outside", false).is_err());
    }

    #[test]
    fn workspace_case_change_preserves_a_distinct_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("draft.tex");
        let target = dir.path().join("Draft.tex");
        std::fs::write(&source, "first paper").unwrap();
        if target.exists() {
            rename_item(&source, &target).unwrap();
            let entry = std::fs::read_dir(dir.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(entry.file_name(), "Draft.tex");
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "first paper");
        } else {
            std::fs::write(&target, "second paper").unwrap();
            assert!(rename_item(&source, &target).is_err());
            assert_eq!(std::fs::read_to_string(&source).unwrap(), "first paper");
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "second paper");
        }
        let other = dir.path().join("another.tex");
        std::fs::write(&other, "another paper").unwrap();
        assert!(rename_without_replace(&other, &target).is_err());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "another paper");
        assert_ne!(std::fs::read_to_string(&target).unwrap(), "another paper");
    }
}
