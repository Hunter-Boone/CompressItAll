//! Engine commands: capabilities, inputs, preview, run, cancel, artifact lookup, trim frames.

use super::blocking;
use crate::state::AppState;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use cia_core::*;
use cia_engine::inspect::InputSpec;
use cia_engine::{CancelToken, PlanRequest};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn capabilities(state: State<'_, AppState>) -> Capabilities {
    state.engine().caps.clone()
}

/// Expand a dropped or picked path list (folders walked in Rust, dot files skipped) into inspected items.
#[tauri::command]
pub async fn add_paths(app: AppHandle, paths: Vec<String>) -> Result<Vec<InputItem>, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let specs = expand(&paths)?;
        let items = state.engine().inspect(&specs);
        let mut map = state.items.lock().unwrap();
        for item in &items {
            if let SourceRef::Path { path } = &item.source {
                map.insert(item.id.clone(), PathBuf::from(path));
            }
        }
        Ok(items)
    })
    .await
}

#[tauri::command]
pub fn remove_input(state: State<'_, AppState>, item_id: String) {
    state.items.lock().unwrap().remove(&item_id);
}

#[tauri::command]
pub async fn preview(app: AppHandle, req: PlanRequest) -> Result<Plan, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let cancel = CancelToken::new();
        {
            let mut slot = state.preview_cancel.lock().unwrap();
            slot.cancel();
            *slot = cancel.clone();
        }
        state
            .engine()
            .preview(&req, &cancel)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Start a job. The UI picks the id so it can route events before the thread runs.
#[tauri::command]
pub fn run(app: AppHandle, job_id: String, req: PlanRequest) -> Result<(), String> {
    if job_id.is_empty() || job_id.len() > 64 || !job_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err("bad job id".into());
    }
    let pro = crate::license::is_pro(&app.state::<AppState>());
    crate::engine::start(&app, job_id, req, pro)
}

#[tauri::command]
pub fn cancel(state: State<'_, AppState>, job_id: String) {
    if let Some(j) = state.job.lock().unwrap().as_ref() {
        if j.job_id == job_id {
            j.cancel.cancel();
        }
    }
}

/// Paths for artifact ids, in order; unknown ids are skipped.
#[tauri::command]
pub fn artifact_paths(state: State<'_, AppState>, ids: Vec<String>) -> Vec<String> {
    let map = state.artifacts.lock().unwrap();
    ids.iter()
        .filter_map(|id| map.get(id))
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

/// One JPEG frame at `ms` as a data URL for the trim panel (4.4). None without FFmpeg.
#[tauri::command]
pub async fn preview_frame(
    app: AppHandle,
    item_id: String,
    ms: u64,
) -> Result<Option<String>, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let Some(path) = state.items.lock().unwrap().get(&item_id).cloned() else {
            return Ok(None);
        };
        let Some(installed) = state.ffmpeg.read().unwrap().clone() else {
            return Ok(None);
        };
        match cia_ffmpeg::extract_frame(&installed, &path, ms) {
            Ok(jpeg) => Ok(Some(format!(
                "data:image/jpeg;base64,{}",
                STANDARD.encode(jpeg)
            ))),
            Err(e) => {
                log::debug!("frame at {ms} ms of {}: {e}", path.display());
                Ok(None)
            }
        }
    })
    .await
}

#[tauri::command]
pub fn version() -> serde_json::Value {
    serde_json::json!({
        "app": env!("CARGO_PKG_VERSION"),
        "build": option_env!("CIA_BUILD").unwrap_or(if cfg!(debug_assertions) { "debug" } else { "release" }),
        "engine": env!("CARGO_PKG_VERSION"),
    })
}

/// Dev builds only: the e2e suite adds files without a native drop.
#[cfg(feature = "dev-build")]
#[tauri::command]
pub fn debug_add_paths(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    use tauri::Emitter;
    app.emit("smidge://add-paths", paths)
        .map_err(|e| e.to_string())
}

fn expand(paths: &[String]) -> Result<Vec<InputSpec>, String> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            let folder = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Folder".into());
            for entry in walk(path)? {
                let rel = entry
                    .strip_prefix(path)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                out.push(InputSpec {
                    source: SourceRef::Path {
                        path: entry.to_string_lossy().to_string(),
                    },
                    rel_path: format!("{folder}/{rel}"),
                    folder: Some(folder.clone()),
                });
            }
        } else if path.is_file() {
            out.push(InputSpec {
                source: SourceRef::Path { path: p.clone() },
                rel_path: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone()),
                folder: None,
            });
        } else {
            log::info!("dropped path does not exist: {p}");
        }
    }
    Ok(out)
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in rd.flatten() {
        let p = e.path();
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if p.is_dir() {
            v.extend(walk(&p)?);
        } else if p.is_file() {
            v.push(p);
        }
    }
    v.sort();
    Ok(v)
}
