//! Bounded archive extraction and same-directory atomic file replacement.
use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub fn unique_path(parent: &Path, prefix: &str) -> PathBuf {
    parent.join(format!(
        ".{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("File has no parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = unique_path(parent, "save");
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
            .with_context(|| format!("Could not replace {}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|part| match part {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                !name.contains(':') && !name.ends_with(['.', ' '])
            }
            _ => false,
        })
}

pub struct Staging(pub PathBuf);
impl Staging {
    pub fn new(parent: &Path) -> Result<Self> {
        fs::create_dir_all(parent)?;
        let path = unique_path(parent, "import");
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn extract<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    root: &Path,
) -> Result<()> {
    const BUDGET: u64 = 256 * 1024 * 1024;
    if archive.len() > 2048 {
        bail!("Package contains too many files");
    }
    let mut remaining = BUDGET;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let relative = entry.enclosed_name().context("Unsafe archive path")?;
        if !safe_relative(&relative) || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            bail!("Unsafe archive entry");
        }
        if entry.size() > remaining {
            bail!("Package exceeds the 256 MiB extraction limit");
        }
        let output = root.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        fs::create_dir_all(output.parent().context("Missing asset directory")?)?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(output)?;
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut file)?;
        if copied > remaining {
            bail!("Package exceeds the extraction limit");
        }
        remaining -= copied;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_parent_absolute_and_stream_paths() {
        for name in ["..", "../data", "C:\\data", "assets/a:stream", "assets/a."] {
            assert!(!safe_relative(Path::new(name)), "{name}");
        }
        assert!(safe_relative(Path::new("assets/widget.wasm")));
    }
    #[test]
    fn replaces_existing_file_atomically() {
        let root = Staging::new(&std::env::temp_dir()).unwrap();
        let path = root.0.join("config.yaml");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"new");
    }
}
