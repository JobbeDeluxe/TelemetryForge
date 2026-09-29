use crate::{
    config::{persistence, schema::AppConfig},
    sensors::model::SensorSnapshot,
};
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

#[derive(Clone, Default, serde::Serialize)]
pub struct Diagnostics {
    pub render_ms: f64,
    pub transfer_ms: f64,
    pub transmitted_frames: u64,
    pub skipped_frames: u64,
    pub transferred_bytes: u64,
    pub errors: u64,
    pub fps: f64,
    #[serde(skip)]
    pub frame_times: std::collections::VecDeque<std::time::Instant>,
}

impl Diagnostics {
    pub fn snapshot(&self) -> Self {
        let mut snapshot = self.clone();
        snapshot.fps = self
            .frame_times
            .iter()
            .filter(|time| time.elapsed() < std::time::Duration::from_secs(5))
            .count() as f64
            / 5.0;
        snapshot
    }

    pub fn transmitted(&mut self, bytes: u64) {
        if bytes == 0 {
            self.skipped_frames += 1;
            return;
        }
        self.transmitted_frames += 1;
        self.transferred_bytes += bytes;
        let now = std::time::Instant::now();
        self.frame_times.push_back(now);
        while self
            .frame_times
            .front()
            .is_some_and(|time| now.duration_since(*time).as_secs() >= 5)
        {
            self.frame_times.pop_front();
        }
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn only_transmitted_pixels_count_as_display_frames() {
        let mut metrics = Diagnostics::default();
        metrics.transmitted(0);
        metrics.transmitted(320);
        assert_eq!(metrics.skipped_frames, 1);
        assert_eq!(metrics.transmitted_frames, 1);
        assert_eq!(metrics.transferred_bytes, 320);
        assert_eq!(metrics.snapshot().fps, 0.2);
        metrics.frame_times[0] = std::time::Instant::now() - std::time::Duration::from_secs(6);
        assert_eq!(metrics.snapshot().fps, 0.0);
    }
}

pub struct RenderWorker {
    pub stop: Arc<AtomicBool>,
    pub handle: std::thread::JoinHandle<()>,
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: PathBuf,
    pub config: Arc<RwLock<AppConfig>>,
    pub sensors: Arc<RwLock<SensorSnapshot>>,
    pub worker: Arc<Mutex<Option<RenderWorker>>>,
    pub status: Arc<RwLock<String>>,
    pub scene_revision: Arc<AtomicU64>,
    pub active_screen: Arc<RwLock<Option<String>>>,
    /// Values supplied by external services for `remote_text` widgets. These
    /// deliberately live outside saved screens: a POST updates the display,
    /// never the user's layout file.
    pub remote_texts: Arc<RwLock<HashMap<String, String>>>,
    pub remote_running: Arc<AtomicBool>,
    pub diagnostics: Arc<RwLock<Diagnostics>>,
}

impl AppState {
    pub fn load() -> Self {
        if let Err(error) = crate::superwidgets::ensure_bundled() {
            tracing::error!(error = %format!("{error:#}"), "could not install bundled superwidgets");
        }
        let config_path = persistence::default_config_path();
        let config = match persistence::load_or_create(&config_path) {
            Ok(config) => config,
            Err(error) => {
                tracing::error!(error = %format!("{error:#}"), "could not load configuration");
                AppConfig::default()
            }
        };
        let active_screen = persistence::load_active_screen(&config_path)
            .or_else(|| persistence::infer_active_screen(&config_path, &config));
        if let Some(name) = active_screen.as_deref() {
            let _ = persistence::save_active_screen(&config_path, Some(name));
        }
        Self {
            config_path,
            config: Arc::new(RwLock::new(config)),
            sensors: Arc::new(RwLock::new(SensorSnapshot::default())),
            worker: Arc::new(Mutex::new(None)),
            status: Arc::new(RwLock::new("Stopped".into())),
            scene_revision: Arc::new(AtomicU64::new(0)),
            active_screen: Arc::new(RwLock::new(active_screen)),
            remote_texts: Arc::new(RwLock::new(HashMap::new())),
            remote_running: Arc::new(AtomicBool::new(false)),
            diagnostics: Arc::new(RwLock::new(Diagnostics::default())),
        }
    }
}
