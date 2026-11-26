use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::storage;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Auto,
    Pdf,
    Xe,
    Lua,
    Tectonic,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Self::Auto | Self::Pdf => "pdflatex",
            Self::Xe => "xelatex",
            Self::Lua => "lualatex",
            Self::Tectonic => "tectonic",
        }
    }

    fn latexmk_flag(self) -> &'static str {
        match self {
            Self::Xe => "-xelatex",
            Self::Lua => "-lualatex",
            _ => "-pdf",
        }
    }
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

pub fn project_source(path: &Path) -> bool {
    ["tex", "ltx", "bib", "sty", "cls", "bst", "bbl"]
        .iter()
        .any(|extension| has_extension(path, extension))
}

pub fn main_file(path: &Path) -> Result<PathBuf, String> {
    let canonical = dunce::canonicalize(path)
        .map_err(|error| format!("Cannot locate LaTeX main file: {error}"))?;
    if !canonical.is_file()
        || !(has_extension(&canonical, "tex") || has_extension(&canonical, "ltx"))
    {
        return Err("Choose a LaTeX main file (.tex or .ltx).".into());
    }
    Ok(canonical)
}

struct Header {
    root: Option<PathBuf>,
    engine: Option<Engine>,
    document: bool,
}

fn header(path: &Path) -> Option<Header> {
    let mut result = Header {
        root: None,
        engine: None,
        document: false,
    };
    for line in BufReader::new(File::open(path).ok()?.take(128 * 1024))
        .lines()
        .take(160)
    {
        let line = line.ok()?;
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some((prefix, value)) = line.split_once('=') {
            let prefix = prefix
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            let value = value.trim();
            match prefix.as_str() {
                "% !tex root" | "%!tex root" => {
                    result.root = Some(PathBuf::from(value.trim_matches(['\'', '"'])))
                }
                "% !tex program" | "%!tex program" | "% !tex ts-program" | "%!tex ts-program" => {
                    result.engine = match value.to_ascii_lowercase().as_str() {
                        "pdflatex" => Some(Engine::Pdf),
                        "xelatex" => Some(Engine::Xe),
                        "lualatex" => Some(Engine::Lua),
                        "tectonic" => Some(Engine::Tectonic),
                        _ => None,
                    };
                }
                _ => {}
            }
        }
        if line.starts_with("\\documentclass")
            || line.starts_with("\\documentstyle")
            || line.starts_with("\\begin{document}")
        {
            result.document = true;
        }
    }
    Some(result)
}

pub fn detect_root(active: &Path, workspace: Option<&Path>) -> Option<PathBuf> {
    if !project_source(active) {
        return None;
    }
    if let Some(info) = header(active) {
        if let Some(root) = info.root {
            return main_file(&active.parent()?.join(root)).ok();
        }
        if info.document {
            return main_file(active).ok();
        }
    }
    let parent = active.parent()?;
    for folder in parent.ancestors().take(12).take_while(|folder| {
        *folder == parent || workspace.is_some_and(|root| folder.starts_with(root))
    }) {
        let mut roots = std::fs::read_dir(folder)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = main_file(&entry.path()).ok()?;
                header(&path)?.document.then_some(path)
            });
        if let Some(root) = roots.next() {
            return roots.next().is_none().then_some(root);
        }
    }
    None
}

fn search_dirs() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    #[cfg(target_os = "macos")]
    paths.extend(["/Library/TeX/texbin", "/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    if let Some(home) = directories::BaseDirs::new() {
        paths.push(home.home_dir().join(".cargo/bin"));
    }
    paths
}

fn executable(paths: &[PathBuf], name: &str) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    paths
        .iter()
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
}

pub struct Build {
    pub main: PathBuf,
    program: PathBuf,
    engine: Engine,
    paths: Vec<PathBuf>,
}

impl Build {
    pub fn prepare(main: PathBuf, engine: Engine) -> Result<Self, String> {
        let main = main_file(&main)?;
        let paths = search_dirs();
        let requested = if engine == Engine::Auto {
            header(&main)
                .and_then(|header| header.engine)
                .unwrap_or(engine)
        } else {
            engine
        };
        let latexmk = || executable(&paths, "latexmk");
        let tectonic = || executable(&paths, "tectonic");
        let (program, engine) = match requested {
            Engine::Tectonic => (tectonic(), Engine::Tectonic),
            Engine::Auto => {
                if let Some(lmk) = latexmk() {
                    if executable(&paths, "pdflatex").is_some() {
                        (Some(lmk), Engine::Pdf)
                    } else if executable(&paths, "xelatex").is_some() {
                        (Some(lmk), Engine::Xe)
                    } else if executable(&paths, "lualatex").is_some() {
                        (Some(lmk), Engine::Lua)
                    } else {
                        (tectonic(), Engine::Tectonic)
                    }
                } else {
                    (tectonic(), Engine::Tectonic)
                }
            }
            engine => (
                latexmk().filter(|_| executable(&paths, engine.name()).is_some()),
                engine,
            ),
        };
        let program = program.ok_or_else(|| if requested == Engine::Auto {
            "No LaTeX compiler found. Install TeX Live, MacTeX or MiKTeX with latexmk, or install Tectonic and restart MarkType.".into()
        } else if requested == Engine::Tectonic {
            "Tectonic was not found. Install it and restart MarkType.".into()
        } else {
            format!("latexmk and {} are required. Install them and restart MarkType.", engine.name())
        })?;
        Ok(Self {
            main,
            program,
            engine,
            paths,
        })
    }

    pub fn compiler(&self) -> &'static str {
        self.engine.name()
    }

    fn command(&self, output: &Path) -> Command {
        let mut command = Command::new(&self.program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        if let Ok(path) = std::env::join_paths(&self.paths) {
            command.env("PATH", path);
        }
        command.current_dir(self.main.parent().expect("main file has a parent"));
        let source = Path::new(".").join(self.main.file_name().expect("main file name"));
        if self.engine == Engine::Tectonic {
            command
                .args(["--untrusted", "--keep-logs", "--synctex", "--outdir"])
                .arg(output)
                .arg(source);
        } else {
            let mut outdir = OsString::from("-outdir=");
            outdir.push(output);
            command
                .args([
                    "-norc",
                    self.engine.latexmk_flag(),
                    "-interaction=nonstopmode",
                    "-halt-on-error",
                    "-file-line-error",
                    "-latexoption=-no-shell-escape",
                    "-latexoption=-synctex=1",
                ])
                .arg(outdir)
                .arg(source);
        }
        command.stdin(Stdio::null());
        command
    }

    pub fn run(self, progress: impl FnMut(String)) -> BuildResult {
        let compiler = self.compiler();
        let main = self.main.clone();
        let (result, log) = self.run_inner(progress);
        match result {
            Ok((pdf, synctex)) => BuildResult {
                main,
                compiler,
                pdf: Some(pdf),
                synctex,
                log,
                error: None,
            },
            Err(error) => BuildResult {
                main,
                compiler,
                pdf: None,
                synctex: false,
                log,
                error: Some(error.to_string()),
            },
        }
    }

    fn run_inner(&self, mut progress: impl FnMut(String)) -> (io::Result<(PathBuf, bool)>, String) {
        let temp = match tempfile::tempdir() {
            Ok(temp) => temp,
            Err(error) => return (Err(error), String::new()),
        };
        let output = temp.path();
        let transcript = output.join("marktype-compiler.txt");
        let pdf = self.main.with_extension("pdf");
        let mut previous_pdf = None;
        let mut result = (|| {
            if self.engine != Engine::Tectonic {
                prepare_output_directories(self.main.parent().expect("main file parent"), output)?;
            }
            let stdout = File::create(&transcript)?;
            let stderr = stdout.try_clone()?;
            previous_pdf = backup_pdf(&pdf)?;
            let mut child = self.command(output).stdout(stdout).stderr(stderr).spawn()?;
            let deadline = Instant::now() + Duration::from_secs(600);
            let mut report_at = Instant::now() + Duration::from_secs(1);
            let mut reported_bytes = 0;
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "LaTeX compilation exceeded 10 minutes",
                    ));
                }
                if Instant::now() >= report_at {
                    let bytes = transcript.metadata().map_or(0, |metadata| metadata.len());
                    if bytes != reported_bytes {
                        progress(tail(&transcript));
                        reported_bytes = bytes;
                    }
                    report_at = Instant::now() + Duration::from_secs(1);
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            if !status.success() {
                return Err(io::Error::other(format!(
                    "LaTeX compilation failed ({status})."
                )));
            }
            let mut compiled = File::open(output.join(pdf.file_name().expect("PDF file name")))?;
            let mut signature = [0; 5];
            compiled.read_exact(&mut signature)?;
            if &signature != b"%PDF-" {
                return Err(io::Error::other("The compiler did not produce a PDF."));
            }
            compiled.rewind()?;
            storage::atomic_write(&pdf, |file| io::copy(&mut compiled, file).map(|_| ()))?;
            let synctex = copy_synctex(output, &self.main).unwrap_or(false);
            Ok((pdf.clone(), synctex))
        })();
        // LuaLaTeX can remove a PDF in the project directory even with an explicit output directory.
        if let (Err(build_error), Some(backup)) = (&result, previous_pdf)
            && let Err(restore) = backup.persist_noclobber(&pdf)
            && restore.error.kind() != io::ErrorKind::AlreadyExists
        {
            let backup_path = restore.file.path().to_path_buf();
            let location = match restore.file.keep() {
                Ok(_) => format!("Backup saved at {}.", backup_path.display()),
                Err(error) => format!("Could not keep the backup: {error}."),
            };
            result = Err(io::Error::other(format!(
                "{build_error} Could not restore the previous PDF: {}. {location}",
                restore.error
            )));
        }
        let mut log = tail(&transcript);
        let engine_log = output.join(
            self.main
                .with_extension("log")
                .file_name()
                .expect("log file name"),
        );
        if result.is_err() && engine_log.exists() {
            log.push_str("\n--- TeX log ---\n");
            log.push_str(&tail(&engine_log));
        }
        (result, log)
    }
}

fn backup_pdf(pdf: &Path) -> io::Result<Option<tempfile::NamedTempFile>> {
    let mut previous = match File::open(pdf) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut backup = tempfile::Builder::new()
        .prefix(".marktype-pdf-")
        .tempfile_in(pdf.parent().expect("PDF parent"))?;
    io::copy(&mut previous, backup.as_file_mut())?;
    #[cfg(unix)]
    backup
        .as_file()
        .set_permissions(previous.metadata()?.permissions())?;
    backup.as_file().sync_all()?;
    Ok(Some(backup))
}

fn prepare_output_directories(project: &Path, output: &Path) -> io::Result<()> {
    // TeX Live writes a distinct .aux file for each \include, under its relative path.
    let staging = dunce::canonicalize(output)?;
    let walker = ignore::WalkBuilder::new(project)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |entry| entry.file_name() != ".git" && entry.path() != staging)
        .build();
    for entry in walker.filter_map(Result::ok) {
        if entry.depth() != 0 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
            let relative = entry
                .path()
                .strip_prefix(project)
                .map_err(io::Error::other)?;
            std::fs::create_dir_all(output.join(relative))?;
        }
    }
    Ok(())
}

fn tail(path: &Path) -> String {
    const LIMIT: u64 = 64 * 1024;
    let read = || -> io::Result<String> {
        let mut file = File::open(path)?;
        let start = file.metadata()?.len().saturating_sub(LIMIT);
        file.seek(SeekFrom::Start(start))?;
        let mut buffer = Vec::new();
        file.take(LIMIT).read_to_end(&mut buffer)?;
        let text = String::from_utf8_lossy(&buffer);
        Ok(if start == 0 {
            text.into_owned()
        } else {
            format!("…\n{text}")
        })
    };
    read().unwrap_or_default()
}

#[derive(Serialize)]
pub struct BuildResult {
    pub main: PathBuf,
    pub compiler: &'static str,
    pub pdf: Option<PathBuf>,
    pub synctex: bool,
    pub log: String,
    pub error: Option<String>,
}

fn copy_synctex(output: &Path, main: &Path) -> io::Result<bool> {
    let gz_target = crate::synctex::with_synctex_extension(main, "synctex.gz");
    let plain_target = crate::synctex::with_synctex_extension(main, "synctex");
    for (target, stale) in [(&gz_target, &plain_target), (&plain_target, &gz_target)] {
        let Some(file_name) = target.file_name() else {
            continue;
        };
        let produced = output.join(file_name);
        if produced.is_file() {
            let result = File::open(produced).and_then(|mut source| {
                storage::atomic_write(target, |file| io::copy(&mut source, file).map(|_| ()))
            });
            let _ = std::fs::remove_file(stale);
            if result.is_err() {
                let _ = std::fs::remove_file(target);
            }
            return result.map(|()| true);
        }
    }
    // Remove stale SyncTeX files if this build produced none.
    let _ = std::fs::remove_file(&gz_target);
    let _ = std::fs::remove_file(&plain_target);
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_deep_includes_in_large_projects() {
        let project = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let deep = Path::new("sections/a/b/c/d/e/f/g/h/i/j");
        std::fs::create_dir_all(project.path().join(deep)).unwrap();
        for i in 0..520 {
            std::fs::create_dir(project.path().join(format!("chapter-{i}"))).unwrap();
        }
        prepare_output_directories(project.path(), output.path()).unwrap();
        assert!(output.path().join(deep).is_dir());
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 521);
    }
}
