//! FFmpeg setup (4.8): manifest, download/verify/install with progress on
//! `ffmpeg://progress`, the user's own copy, remove, re-test, cancel. After any
//! change the engine is rebuilt so rows re-plan with the new capabilities.

use super::blocking;
use crate::state::AppState;
use cia_ffmpeg::install::{InstallError, Manifest};
use cia_ffmpeg::{InstallProgress, InstalledFfmpeg};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

pub const EVENT: &str = "ffmpeg://progress";

/// `FfmpegSetupProgress` in packages/engine-client.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub downloaded_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl From<InstallProgress> for Progress {
    fn from(p: InstallProgress) -> Self {
        let (phase, downloaded_bytes, total_bytes) = match p {
            InstallProgress::Downloading { done, total } => ("downloading", Some(done), total),
            InstallProgress::Checking | InstallProgress::Unpacking => ("checking", None, None),
            InstallProgress::Testing => ("testing", None, None),
            InstallProgress::Ready => ("ready", None, None),
        };
        Progress {
            phase,
            downloaded_bytes,
            total_bytes,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestInfo {
    pub version: String,
    pub bytes: u64,
    pub unpacked_bytes: u64,
}

/// Where the manifest lives: the pinned release, or an override for dev/e2e builds.
fn pin() -> cia_ffmpeg::pinned::Pin {
    #[allow(unused_mut)]
    let mut pin = cia_ffmpeg::pinned::PIN;
    #[cfg(feature = "dev-build")]
    {
        // Leaked once per process: `Pin` holds `&'static str`.
        if let Ok(m) = std::env::var("CIA_FFMPEG_MANIFEST_URL") {
            pin.manifest_url = Box::leak(m.into_boxed_str());
        }
        if let Ok(s) = std::env::var("CIA_FFMPEG_SIGNATURE_URL") {
            pin.signature_url = Box::leak(s.into_boxed_str());
        }
    }
    pin
}

/// The download size shown in the modal comes from the manifest, never a constant.
#[tauri::command]
pub async fn ffmpeg_manifest() -> Result<ManifestInfo, String> {
    blocking(move || {
        let pin = pin();
        let bytes = if let Some(path) = pin.manifest_url.strip_prefix("file://") {
            std::fs::read(path).map_err(|e| e.to_string())?
        } else {
            reqwest::blocking::Client::builder()
                .user_agent(concat!("Smidge/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|e| e.to_string())?
                .get(pin.manifest_url)
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes())
                .map_err(|e| e.to_string())?
                .to_vec()
        };
        let m: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let asset = m
            .asset_for_this_platform()
            .ok_or_else(|| "no FFmpeg build for this platform".to_string())?;
        Ok(ManifestInfo {
            version: m.version.clone(),
            bytes: asset.bytes,
            unpacked_bytes: asset.unpacked_bytes,
        })
    })
    .await
}

#[tauri::command]
pub async fn ffmpeg_install(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let cancel = cia_ffmpeg::Cancel::new();
        *state.ffmpeg_cancel.lock().unwrap() = Some(cancel.clone());
        let emit = |p: Progress| {
            if let Err(e) = app.emit(EVENT, &p) {
                log::warn!("emit {EVENT}: {e}");
            }
        };
        let mut progress = |p: InstallProgress| emit(Progress::from(p));
        let fetcher = cia_ffmpeg::install::HttpFetcher::new().map_err(|e| e.user_message(false))?;
        let result = cia_ffmpeg::install::install_with(
            &fetcher,
            &pin(),
            &state.paths.app_data,
            &mut progress,
            &cancel,
        );
        *state.ffmpeg_cancel.lock().unwrap() = None;
        match result {
            Ok(installed) => {
                state.integrity_failures.store(0, Ordering::SeqCst);
                let _ = crate::settings::set(&state.paths, "ffmpegPath", None);
                emit(Progress::from(InstallProgress::Testing));
                adopt(&state, installed)?;
                emit(Progress::from(InstallProgress::Ready));
                Ok(())
            }
            Err(e) => Err(install_error(&state, e)),
        }
    })
    .await
}

fn install_error(state: &AppState, e: InstallError) -> String {
    let repeat = if e.is_integrity_failure() {
        state.integrity_failures.fetch_add(1, Ordering::SeqCst) >= 1
    } else {
        false
    };
    log::warn!("ffmpeg install failed: {e}");
    e.user_message(repeat)
}

/// Run the encoder tests and rebuild the engine with this copy.
fn adopt(state: &AppState, installed: InstalledFfmpeg) -> Result<(), String> {
    log::info!(
        "FFmpeg {} at {} ({})",
        installed.version,
        installed.ffmpeg.display(),
        if installed.user_supplied {
            "user copy"
        } else {
            "Smidge install"
        }
    );
    crate::engine::rebuild(state, Some(installed));
    if state.engine().caps.ffmpeg.is_none() {
        crate::engine::rebuild(state, None);
        return Err(
            "Smidge couldn't set up FFmpeg. Please try again, or use your own copy.".to_string(),
        );
    }
    Ok(())
}

/// "Use my own copy…": `ffprobe` must sit next to the chosen `ffmpeg`.
#[tauri::command]
pub async fn ffmpeg_use_own_copy(app: AppHandle, path: String) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let installed = cia_ffmpeg::use_own_copy(Path::new(&path)).map_err(|e| {
            log::warn!("own copy {path}: {e}");
            match e {
                InstallError::NotWorking(m) if m.contains("ffprobe") => {
                    "Smidge needs ffprobe next to ffmpeg. Choose a folder that has both."
                        .to_string()
                }
                other => other.user_message(false),
            }
        })?;
        crate::settings::set(
            &state.paths,
            "ffmpegPath",
            Some(serde_json::Value::from(
                installed.ffmpeg.to_string_lossy().to_string(),
            )),
        )
        .map_err(|e| e.to_string())?;
        adopt(&state, installed)
    })
    .await
}

/// Remove a Smidge install (a user's own copy is only forgotten, never deleted).
#[tauri::command]
pub async fn ffmpeg_remove(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        if state.job_running() {
            return Err("Wait for the current job to finish before removing video support.".into());
        }
        let current = state.ffmpeg.read().unwrap().clone();
        if let Some(installed) = current {
            cia_ffmpeg::remove(&installed).map_err(|e| format!("could not remove FFmpeg: {e}"))?;
        }
        let _ = crate::settings::set(&state.paths, "ffmpegPath", None);
        crate::engine::rebuild(&state, None);
        Ok(())
    })
    .await
}

/// Forget the cached encoder tests and run them again (3.5.6).
#[tauri::command]
pub async fn ffmpeg_retest(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let Some(installed) = state.ffmpeg.read().unwrap().clone() else {
            return Err("Video support isn't set up yet.".into());
        };
        let cache = installed.encoders_cache_path(&state.paths.app_data);
        let _ = std::fs::remove_file(cache);
        adopt(&state, installed)
    })
    .await
}

#[tauri::command]
pub fn ffmpeg_cancel(state: State<'_, AppState>) {
    if let Some(c) = state.ffmpeg_cancel.lock().unwrap().as_ref() {
        c.cancel();
    }
}
