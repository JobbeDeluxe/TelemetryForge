pub mod renderer;
mod wasm_runtime;

use crate::config::persistence;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
};
use zip::ZipArchive;

const CPU_MANIFEST: &str = include_str!("../../superwidgets/cpu-command-dial/manifest.yaml");
const GPU_MANIFEST: &str = include_str!("../../superwidgets/gpu-command-dial/manifest.yaml");
const HELLO_MANIFEST: &str = include_str!("../../sdk/examples/hello-dial/manifest.yaml");
const HELLO_WASM: &[u8] = include_bytes!("../../sdk/examples/hello-dial/widget.wasm");
const REACTOR_MANIFEST: &str = include_str!("../../sdk/examples/reactor-core/manifest.yaml");
const REACTOR_WASM: &[u8] = include_bytes!("../../sdk/examples/reactor-core/widget.wasm");
const COMMAND_DIALS_SCREEN: &str =
    include_str!("../../samples/superwidgets/screens/Command Dials.yaml");
const REACTOR_SCREEN: &str = include_str!("../../samples/superwidgets/screens/Reactor Core.yaml");
const HELLO_SCREEN: &str = include_str!("../../samples/superwidgets/screens/SDK Hello Dial.yaml");
const BOOT_SCREEN: &str =
    include_str!("../../samples/superwidgets/screens/TelemetryForge Boot.yaml");

static REGISTRY: LazyLock<RwLock<Option<Vec<Manifest>>>> = LazyLock::new(|| RwLock::new(None));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub template: String,
    #[serde(default = "native_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub entry: Option<String>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub sensors: Vec<String>,
    #[serde(default = "abi_version")]
    pub abi_version: u32,
    #[serde(default)]
    pub animated_fps: u16,
}

fn native_runtime() -> String {
    "native".into()
}

const fn abi_version() -> u32 {
    1
}

pub fn ensure_bundled() -> Result<()> {
    for (id, contents) in [
        ("cpu-command-dial", CPU_MANIFEST),
        ("gpu-command-dial", GPU_MANIFEST),
    ] {
        let folder = directory().join(id);
        fs::create_dir_all(&folder)?;
        write_bundled(&folder.join("manifest.yaml"), contents.as_bytes())?;
    }
    for (id, manifest, module) in [
        ("sdk.hello-dial", HELLO_MANIFEST, HELLO_WASM),
        ("sdk.reactor-core", REACTOR_MANIFEST, REACTOR_WASM),
    ] {
        let folder = directory().join(id);
        fs::create_dir_all(&folder)?;
        write_bundled(&folder.join("manifest.yaml"), manifest.as_bytes())?;
        write_bundled(&folder.join("widget.wasm"), module)?;
    }
    let screens = persistence::data_dir().join("screens");
    fs::create_dir_all(&screens)?;
    for (name, contents) in [
        ("Command Dials.yaml", COMMAND_DIALS_SCREEN),
        ("Reactor Core.yaml", REACTOR_SCREEN),
        ("SDK Hello Dial.yaml", HELLO_SCREEN),
        ("TelemetryForge Boot.yaml", BOOT_SCREEN),
    ] {
        let path = screens.join(name);
        if !path.exists() {
            fs::write(path, contents)?;
        }
    }
    Ok(())
}

pub fn list() -> Result<Vec<Manifest>> {
    if let Some(cached) = REGISTRY.read().expect("registry poisoned").as_ref() {
        return Ok(cached.clone());
    }
    let mut manifests: Vec<Manifest> = Vec::new();
    for entry in fs::read_dir(directory())?.flatten() {
        let path = entry.path().join("manifest.yaml");
        if !path.is_file() {
            continue;
        }
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(manifest) = serde_yaml::from_str::<Manifest>(&text) {
                if validate_manifest(&manifest).is_ok() {
                    manifests.push(manifest);
                }
            }
        }
    }
    manifests.sort_by_key(|manifest| manifest.name.to_lowercase());
    *REGISTRY.write().expect("registry poisoned") = Some(manifests.clone());
    Ok(manifests)
}

pub fn find(id: &str) -> Option<Manifest> {
    if let Some(cached) = REGISTRY.read().ok()?.as_ref() {
        return cached.iter().find(|manifest| manifest.id == id).cloned();
    }
    list().ok()?.into_iter().find(|manifest| manifest.id == id)
}

fn write_bundled(path: &Path, contents: &[u8]) -> Result<()> {
    if !path.exists() {
        crate::safe_files::atomic_write(path, contents)?;
    }
    Ok(())
}

pub fn component_dir(id: &str) -> PathBuf {
    directory().join(id)
}

pub fn export_bytes(id: &str) -> Result<Vec<u8>> {
    use std::io::{Cursor, Write};
    use zip::{write::SimpleFileOptions, ZipWriter};
    let manifest = find(id).ok_or_else(|| anyhow::anyhow!("Super Widget not installed: {id}"))?;
    validate_manifest(&manifest)?;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.yaml", options)?;
    zip.write_all(serde_yaml::to_string(&manifest)?.as_bytes())?;
    if manifest.runtime == "wasm" {
        let entry = manifest.entry.as_deref().unwrap_or("widget.wasm");
        let path = component_dir(id).join(entry);
        anyhow::ensure!(
            fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
            "WASM module exceeds 16 MiB"
        );
        zip.start_file(entry, options)?;
        std::io::copy(&mut File::open(path)?, &mut zip)?;
    }
    Ok(zip.finish()?.into_inner())
}

pub fn install(package: &Path) -> Result<Manifest> {
    let mut archive = ZipArchive::new(File::open(package)?)?;
    let staging = crate::safe_files::Staging::new(&directory())?;
    crate::safe_files::extract(&mut archive, &staging.0)?;
    let manifest: Manifest =
        serde_yaml::from_str(&fs::read_to_string(staging.0.join("manifest.yaml"))?)?;
    validate_manifest(&manifest)?;
    let destination = component_dir(&manifest.id);
    let entry = manifest.entry.as_deref().unwrap_or("widget.wasm");
    if manifest.runtime == "wasm" {
        wasm_runtime::validate_module(&staging.0.join(entry))?;
    }
    let backup = crate::safe_files::unique_path(&directory(), "previous");
    let had_previous = destination.exists();
    if had_previous {
        fs::rename(&destination, &backup)?;
    }
    if let Err(error) = fs::rename(&staging.0, &destination) {
        if had_previous {
            let _ = fs::rename(&backup, &destination);
        }
        return Err(error.into());
    }
    if had_previous {
        let _ = fs::remove_dir_all(backup);
    }
    *REGISTRY.write().expect("registry poisoned") = None;
    Ok(manifest)
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    if manifest.id.is_empty()
        || manifest.id == "."
        || manifest.id == ".."
        || manifest.id.ends_with('.')
        || manifest.id.len() > 100
        || manifest.id.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
        })
    {
        anyhow::bail!("Invalid Super Widget id");
    }
    if !crate::safe_files::safe_relative(Path::new(
        manifest.entry.as_deref().unwrap_or("widget.wasm"),
    )) {
        anyhow::bail!("Invalid Super Widget entry path");
    }
    if manifest.width == 0
        || manifest.height == 0
        || manifest.width > 2048
        || manifest.height > 2048
    {
        anyhow::bail!("Super Widget dimensions must be between 1 and 2048");
    }
    if !matches!(manifest.runtime.as_str(), "native" | "wasm") {
        anyhow::bail!("Unsupported Super Widget runtime");
    }
    if manifest.abi_version != 1 {
        anyhow::bail!(
            "Unsupported Super Widget ABI version {}",
            manifest.abi_version
        );
    }
    Ok(())
}

pub fn directory() -> PathBuf {
    persistence::data_dir().join("superwidgets")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_destructive_ids_and_outside_entry_paths() {
        let mut manifest: Manifest = serde_yaml::from_str(HELLO_MANIFEST).unwrap();
        for id in [".", "..", "../outside", "C:\\outside", "bad."] {
            manifest.id = id.into();
            assert!(validate_manifest(&manifest).is_err(), "{id}");
        }
        manifest.id = "valid.widget".into();
        for entry in ["../outside.wasm", "C:\\outside.wasm", "/outside.wasm"] {
            manifest.entry = Some(entry.into());
            assert!(validate_manifest(&manifest).is_err(), "{entry}");
        }
    }
    #[test]
    fn bundled_install_does_not_overwrite_existing_files() {
        let root = crate::safe_files::Staging::new(&std::env::temp_dir()).unwrap();
        let path = root.0.join("widget.wasm");
        fs::write(&path, b"custom").unwrap();
        write_bundled(&path, b"bundled").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"custom");
    }
}
