use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) fn document_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "mdown" | "mkd" | "txt" | "tex")
    )
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
}
