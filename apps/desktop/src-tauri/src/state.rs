//! Process-wide state: the engine, the running job, id-to-path maps, the
//! persisted `state.json`, and the long-lived helpers (HTTP client, clipboard).

use crate::paths::{write_atomic, AppPaths};
use cia_engine::{CancelToken, Engine};
use cia_ffmpeg::InstalledFfmpeg;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, RwLock};

/// `<app_data>/state.json` (DESIGN.md 3.10, 5.5).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopState {
    /// License clock state (5.5): largest clock value seen, last refresh.
    #[serde(flatten)]
    pub license: cia_license::LicenseState,
    /// Output folders of the last 50 jobs, newest last; partial files are swept here at launch.
    pub recent_output_dirs: Vec<String>,
    /// Shown in Settings → License; refreshed by activate and refresh.
    pub key_masked: Option<String>,
    pub devices_used: Option<u32>,
    pub devices_max: Option<u32>,
    /// A refresh answered revoked / ended / deactivated; shown once, then cleared.
    pub license_notice: Option<String>,
    /// Random id for the updater rollout (6.3); unrelated to the license device hash.
    pub install_id: Option<String>,
}

impl DesktopState {
    pub fn load(paths: &AppPaths) -> Self {
        std::fs::read(paths.state())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &AppPaths) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            if let Err(e) = write_atomic(&paths.state(), &bytes) {
                log::warn!("could not write state.json: {e}");
            }
        }
    }

    pub fn remember_output_dir(&mut self, dir: &str) {
        self.recent_output_dirs.retain(|d| d != dir);
        self.recent_output_dirs.push(dir.to_string());
        let extra = self.recent_output_dirs.len().saturating_sub(50);
        if extra > 0 {
            self.recent_output_dirs.drain(..extra);
        }
    }
}

pub struct RunningJob {
    pub job_id: String,
    pub cancel: CancelToken,
}

pub struct AppState {
    pub paths: AppPaths,
    pub engine: RwLock<Arc<Engine>>,
    pub ffmpeg: RwLock<Option<InstalledFfmpeg>>,
    pub desktop: Mutex<DesktopState>,
    /// Item id → source path, filled by `add_paths`, used by the trim preview.
    pub items: Mutex<HashMap<String, PathBuf>>,
    /// Artifact id → written path, filled when a job finishes.
    pub artifacts: Mutex<HashMap<String, PathBuf>>,
    pub job: Mutex<Option<RunningJob>>,
    /// The previous preview is cancelled when a new one starts.
    pub preview_cancel: Mutex<CancelToken>,
    pub ffmpeg_cancel: Mutex<Option<cia_ffmpeg::Cancel>>,
    /// Consecutive hash/signature failures of the FFmpeg download (4.8).
    pub integrity_failures: AtomicU32,
    pub clipboard: Mutex<Option<clipboard_rs::ClipboardContext>>,
    /// Claim secrets by claim id for the purchase flow (5.6.1).
    pub claims: Mutex<HashMap<String, String>>,
    pub pending_update: Mutex<Option<tauri_plugin_updater::Update>>,
}

impl AppState {
    pub fn engine(&self) -> Arc<Engine> {
        self.engine.read().unwrap().clone()
    }

    pub fn job_running(&self) -> bool {
        self.job.lock().unwrap().is_some()
    }

    pub fn with_desktop<T>(&self, f: impl FnOnce(&mut DesktopState) -> T) -> T {
        let mut d = self.desktop.lock().unwrap();
        let out = f(&mut d);
        d.save(&self.paths);
        out
    }
}
