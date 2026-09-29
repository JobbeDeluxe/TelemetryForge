use crate::config::{
    persistence,
    schema::{AppConfig, BackgroundSource},
};
use anyhow::{Context, Result};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

pub fn export(path: &Path, config: &AppConfig) -> Result<()> {
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    config.validate()?;
    let mut portable = config.clone();
    portable.remote = Default::default();
    portable.libre_hardware_monitor_dll = None;

    if let Some(source) = portable.background.path.clone() {
        portable.background.path = add_asset(&mut zip, &source, "background", options)?;
    }
    if portable.background.source == BackgroundSource::Folder {
        if let Some(folder) = portable.background.folder.clone() {
            for entry in fs::read_dir(&folder)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_file()
                    && path.extension().is_some_and(|ext| {
                        matches!(
                            ext.to_string_lossy().to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "bmp" | "gif"
                        )
                    })
                {
                    let name = format!("slideshow/{}", entry.file_name().to_string_lossy());
                    zip.start_file(name, options)?;
                    std::io::copy(&mut File::open(path)?, &mut zip)?;
                }
            }
            portable.background.folder = Some("slideshow".into());
        }
    }
    let mut components = std::collections::HashSet::new();
    for (index, widget) in portable.widgets.iter_mut().enumerate() {
        if let Some(source) = widget.gif_path.clone() {
            widget.gif_path = add_asset(&mut zip, &source, &format!("gif-{index}"), options)?;
        }
        // Only explicitly selected font files are embedded. Installed Windows
        // fonts keep their family name and are not redistributed implicitly.
        if Path::new(&widget.font).is_file() {
            widget.font = add_asset(&mut zip, &widget.font, &format!("font-{index}"), options)?
                .unwrap_or_default();
        }
        if let Some(id) = widget.superwidget_id.as_deref() {
            if components.insert(id.to_string()) {
                let bytes = crate::superwidgets::export_bytes(id)?;
                zip.start_file(format!("components/{id}.superwidget"), options)?;
                zip.write_all(&bytes)?;
            }
        }
    }

    zip.start_file("config.yaml", options)?;
    zip.write_all(serde_yaml::to_string(&portable)?.as_bytes())?;
    zip.finish()?;
    Ok(())
}

pub fn import(path: &Path, name: &str) -> Result<AppConfig> {
    let parent = persistence::data_dir().join("packages");
    let staging = crate::safe_files::Staging::new(&parent)?;
    let destination = crate::safe_files::unique_path(&parent, &safe_name(name));
    let mut archive = ZipArchive::new(File::open(path)?)?;
    crate::safe_files::extract(&mut archive, &staging.0)?;
    let mut config: AppConfig = serde_yaml::from_str(
        &fs::read_to_string(staging.0.join("config.yaml"))
            .context("Package does not contain config.yaml")?,
    )?;
    config.validate()?;
    resolve_asset(&staging.0, &destination, &mut config.background.path)?;
    resolve_asset(&staging.0, &destination, &mut config.background.folder)?;
    for widget in &mut config.widgets {
        resolve_asset(&staging.0, &destination, &mut widget.gif_path)?;
        if widget.font.starts_with("assets/") {
            let mut font = Some(widget.font.clone());
            resolve_asset(&staging.0, &destination, &mut font)?;
            widget.font = font.unwrap_or_default();
        }
    }
    let components = staging.0.join("components");
    if components.is_dir() {
        for entry in fs::read_dir(components)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "superwidget") {
                crate::superwidgets::install(&path)?;
            }
        }
    }
    fs::rename(&staging.0, &destination)?;
    config.remote = Default::default();
    Ok(config)
}

fn add_asset(
    zip: &mut ZipWriter<File>,
    source: &str,
    name: &str,
    options: SimpleFileOptions,
) -> Result<Option<String>> {
    let path = Path::new(source);
    if !path.is_file() {
        return Ok(None);
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("bin");
    let archive_path = format!("assets/{name}.{extension}");
    zip.start_file(&archive_path, options)?;
    let mut source_file = File::open(path)?;
    let mut bytes = Vec::new();
    source_file.read_to_end(&mut bytes)?;
    zip.write_all(&bytes)?;
    Ok(Some(archive_path))
}

fn resolve_asset(staging: &Path, root: &Path, value: &mut Option<String>) -> Result<()> {
    if let Some(path) = value.as_deref() {
        anyhow::ensure!(
            crate::safe_files::safe_relative(Path::new(path)),
            "Unsafe package asset path"
        );
        anyhow::ensure!(
            staging.join(path).exists(),
            "Package asset is missing: {path}"
        );
        *value = Some(root.join(path).to_string_lossy().into_owned());
    }
    Ok(())
}

fn safe_name(name: &str) -> String {
    let clean = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if clean.is_empty() {
        "imported".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{BackgroundSource, WidgetConfig, WidgetKind};

    #[test]
    fn package_round_trip_embeds_background_and_gif() {
        let root = std::env::temp_dir().join(format!(
            "telemetryforge-package-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let background = root.join("background.png");
        let gif = root.join("animation.gif");
        fs::write(&background, b"background-bytes").unwrap();
        fs::write(&gif, b"gif-bytes").unwrap();
        let package = root.join("sample.telemetryforge");

        let mut config = AppConfig::default();
        config.remote.password_hash = "must-not-be-exported".into();
        config.remote.text_token_hash = "must-not-be-exported-either".into();
        config.background.source = BackgroundSource::File;
        config.background.path = Some(background.to_string_lossy().into_owned());
        let mut widget = WidgetConfig::new(WidgetKind::Gif, 0, 0, "");
        widget.gif_path = Some(gif.to_string_lossy().into_owned());
        config.widgets = vec![widget];

        export(&package, &config).unwrap();
        let imported = import(&package, "package-test").unwrap();
        assert!(imported.remote.password_hash.is_empty());
        assert!(imported.remote.text_token_hash.is_empty());
        assert!(Path::new(imported.background.path.as_deref().unwrap()).is_file());
        assert!(Path::new(imported.widgets[0].gif_path.as_deref().unwrap()).is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn package_round_trip_embeds_slideshow_and_custom_font() {
        let root = crate::safe_files::Staging::new(&std::env::temp_dir()).unwrap();
        let slides = root.0.join("slides");
        fs::create_dir(&slides).unwrap();
        fs::write(slides.join("one.png"), b"slide-bytes").unwrap();
        let font = root.0.join("custom.ttf");
        fs::write(&font, b"font-bytes").unwrap();
        let mut config = AppConfig::default();
        config.background.source = BackgroundSource::Folder;
        config.background.folder = Some(slides.to_string_lossy().into_owned());
        config.widgets[0].font = font.to_string_lossy().into_owned();
        let package = root.0.join("slides.telemetryforge");
        export(&package, &config).unwrap();
        let imported = import(&package, "slides-test").unwrap();
        assert_eq!(
            fs::read(Path::new(imported.background.folder.as_deref().unwrap()).join("one.png"))
                .unwrap(),
            b"slide-bytes"
        );
        assert_eq!(fs::read(&imported.widgets[0].font).unwrap(), b"font-bytes");
    }
}
