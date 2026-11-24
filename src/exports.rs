use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::engine::Document;
use crate::storage;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PandocFormat {
    Docx,
    Tex,
}

impl PandocFormat {
    pub(crate) fn extension(&self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Tex => "tex",
        }
    }

    fn writer(&self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Tex => "latex",
        }
    }
}

pub(crate) fn pandoc() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "pandoc.exe"
    } else {
        "pandoc"
    };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find_map(|path| dunce::canonicalize(path).ok().filter(|path| path.is_file()))
}

pub(crate) fn convert_document(
    binary: &Path,
    doc: &Document,
    dest: &Path,
    format: PandocFormat,
) -> Result<(), String> {
    let convert = || -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let input = temp.path().join("source.md");
        let output = temp
            .path()
            .join("converted")
            .with_extension(format.extension());
        doc.write_text(std::fs::File::create(&input)?)?;
        let mut command = std::process::Command::new(binary);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(directory) = doc.path.as_deref().and_then(Path::parent) {
            command.current_dir(directory);
        }
        let result = command
            .args([
                "--from=markdown",
                "--standalone",
                "--to",
                format.writer(),
                "--output",
            ])
            .arg(&output)
            .arg(&input)
            .output()?;
        if !result.status.success() {
            return Err(String::from_utf8_lossy(&result.stderr)
                .chars()
                .take(400)
                .collect::<String>()
                .into());
        }
        storage::atomic_write(dest, |file| {
            std::io::copy(&mut std::fs::File::open(&output)?, file).map(|_| ())
        })?;
        Ok(())
    };
    convert().map_err(|error| format!("Pandoc export failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_pandoc_converts_the_editor_buffer_to_word_and_latex() {
        let Some(pandoc) = pandoc() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let doc = Document::new(
            "1".into(),
            "Note.md".into(),
            None,
            "# MarkType\n\n**Unsaved text**",
        );
        let word = dir.path().join("note.docx");
        convert_document(&pandoc, &doc, &word, PandocFormat::Docx).unwrap();
        assert!(std::fs::read(word).unwrap().starts_with(b"PK"));
        let tex = dir.path().join("note.tex");
        convert_document(&pandoc, &doc, &tex, PandocFormat::Tex).unwrap();
        let output = std::fs::read_to_string(tex).unwrap();
        assert!(output.contains("\\documentclass"));
        assert!(output.contains("\\textbf{Unsaved text}"));
    }
}
