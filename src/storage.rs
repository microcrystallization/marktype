use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub(crate) type Fingerprint = [u8; 32];

pub(crate) fn fingerprint(bytes: &[u8]) -> Fingerprint {
    *blake3::hash(bytes).as_bytes()
}

pub(crate) fn disk_fingerprint(path: &Path) -> io::Result<Option<Fingerprint>> {
    match File::open(path) {
        Ok(file) => {
            let mut hasher = blake3::Hasher::new();
            hasher.update_reader(file)?;
            Ok(Some(*hasher.finalize().as_bytes()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct FileStamp {
    modified: Option<SystemTime>,
    size: u64,
}

impl FileStamp {
    fn from_metadata(metadata: std::fs::Metadata) -> Self {
        Self {
            modified: metadata.modified().ok(),
            size: metadata.len(),
        }
    }

    pub(crate) fn read(path: &Path) -> io::Result<Option<Self>> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(Some(Self::from_metadata(metadata))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

pub(crate) fn read_document(path: &Path) -> io::Result<(String, FileStamp)> {
    let mut file = File::open(path)?;
    let stamp = FileStamp::from_metadata(file.metadata()?);
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok((text, stamp))
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct Session {
    pub(crate) tabs: Vec<SessionTab>,
    pub(crate) active: Option<usize>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct SessionTab {
    pub(crate) path: Option<std::path::PathBuf>,
    pub(crate) title: String,
    pub(crate) draft: Option<String>,
    pub(crate) expected_disk: Option<Fingerprint>,
    #[serde(default)]
    pub(crate) manual_save: bool,
}

pub(crate) fn lock_session(path: &Path) -> io::Result<File> {
    let lease = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    fs2::FileExt::try_lock_exclusive(&lease)?;
    Ok(lease)
}

pub(crate) fn claim_session(directory: &Path) -> io::Result<(std::path::PathBuf, File)> {
    std::fs::create_dir_all(directory)?;
    let primary = directory.join("session.json");
    if let Ok(lease) = lock_session(&primary) {
        return Ok((primary, lease));
    }
    let temp = tempfile::Builder::new()
        .prefix("session-extra-")
        .suffix(".lock")
        .tempfile_in(directory)?;
    let (lease, lock_path) = temp.keep().map_err(|error| error.error)?;
    if let Err(error) = fs2::FileExt::try_lock_exclusive(&lease) {
        drop(lease);
        let _ = std::fs::remove_file(&lock_path);
        return Err(error);
    }
    Ok((lock_path.with_extension("json"), lease))
}

pub(crate) fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    atomic_write_checked(path, write, || Ok(()))
}

pub(crate) fn atomic_write_checked(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
    check: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    write(temp.as_file_mut())?;
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.as_file().sync_all()?;
    check()?;
    #[cfg(windows)]
    {
        let mut current = temp;
        let mut attempts = 0u8;
        loop {
            match current.persist(path) {
                Ok(_) => break,
                Err(error)
                    if error.error.kind() == io::ErrorKind::PermissionDenied && attempts < 4 =>
                {
                    attempts += 1;
                    current = error.file;
                    std::thread::sleep(std::time::Duration::from_millis(20 * u64::from(attempts)));
                }
                Err(error) => return Err(error.error),
            }
        }
    }
    #[cfg(not(windows))]
    temp.persist(path)?;
    Ok(())
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, |file| {
        serde_json::to_writer(file, value).map_err(io::Error::other)
    })
}
