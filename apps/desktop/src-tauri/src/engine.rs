//! Building the engine for this host and running jobs on a worker thread
//! (DESIGN.md 2.2, 3.12): one job at a time, events on `engine://event`,
//! job logs under `<app_data>/logs/jobs/`, the free allowance consumed per
//! `Fitted` item.

use crate::paths::{stamp, write_atomic, AppPaths};
use crate::state::{AppState, RunningJob};
use cia_core::allowance::AllowanceState;
use cia_core::events::{EngineEvent, EventSink};
use cia_core::*;
use cia_engine::output::fs_sink::{FsReader, FsSink};
use cia_engine::video_ffmpeg::FfmpegBackend;
use cia_engine::{CancelToken, Engine, PlanRequest, VideoBackend};
use cia_ffmpeg::InstalledFfmpeg;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT: &str = "engine://event";
const JOB_LOGS_KEPT: usize = 200;

/// Build the engine with the filesystem reader and sink, and FFmpeg when one is installed.
pub fn build(paths: &AppPaths, ffmpeg: Option<&InstalledFfmpeg>) -> Engine {
    let caps = Capabilities {
        host: "desktop".into(),
        can_copy_files: true,
        can_drag_out: cfg!(feature = "drag"),
        can_reveal: true,
        can_choose_folder: true,
        opus_encode: true,
        ..Default::default()
    };
    let mut e = Engine::new(Arc::new(FsReader), Arc::new(FsSink::new("idle")), caps);
    e.temp_dir = Some(paths.temp_dir().to_string_lossy().to_string());
    if let Some(installed) = ffmpeg {
        match FfmpegBackend::from_installed(installed.clone(), &paths.app_data) {
            Ok(ff) => {
                let caps = ff.capabilities();
                e = e.with_video(Arc::new(ff));
                if let Some(c) = caps {
                    e.caps.mp3_encode = c.has_libmp3lame;
                    e.caps.aac_encode = c.has_aac;
                    e.caps.heic_input = true;
                    e.caps.avif_input = true;
                }
            }
            Err(err) => log::warn!("FFmpeg at {:?} is not usable: {err}", installed.ffmpeg),
        }
    }
    e
}

/// Find FFmpeg: the user's own copy from settings first, then a Smidge install (or `CIA_FFMPEG`).
pub fn locate_ffmpeg(paths: &AppPaths) -> Option<InstalledFfmpeg> {
    if let Some(own) = crate::settings::get_str(paths, "ffmpegPath") {
        match cia_ffmpeg::use_own_copy(Path::new(&own)) {
            Ok(i) => return Some(i),
            Err(e) => log::warn!("own FFmpeg copy {own} is not usable: {e}"),
        }
    }
    cia_ffmpeg::locate(&paths.app_data)
}

/// Replace the engine after FFmpeg was installed, removed or re-tested.
pub fn rebuild(state: &AppState, ffmpeg: Option<InstalledFfmpeg>) {
    let engine = build(&state.paths, ffmpeg.as_ref());
    *state.ffmpeg.write().unwrap() = ffmpeg;
    *state.engine.write().unwrap() = Arc::new(engine);
}

/// Emits to the webview, throttles progress to 10 per second per item, and
/// records what the host needs from the outcomes.
struct TauriSink {
    app: AppHandle,
    pro: bool,
    last_progress: Mutex<HashMap<String, Instant>>,
}

impl EventSink for TauriSink {
    fn emit(&self, event: EngineEvent) {
        match &event {
            EngineEvent::Progress {
                item_id, fraction, ..
            } => {
                let mut last = self.last_progress.lock().unwrap();
                let now = Instant::now();
                let due = last
                    .get(item_id)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_millis(100));
                if !due && *fraction < 1.0 {
                    return;
                }
                last.insert(item_id.clone(), now);
            }
            EngineEvent::ItemDone {
                outcome: ItemOutcome::Fitted { .. },
                ..
            } if !self.pro => {
                let state = self.app.state::<AppState>();
                let mut a = load_allowance(&state.paths);
                a.consume(crate::paths::now_ms());
                save_allowance(&state.paths, &a);
            }
            _ => {}
        }
        if let Err(e) = self.app.emit(EVENT, &event) {
            log::warn!("emit {EVENT}: {e}");
        }
    }
}

pub fn load_allowance(paths: &AppPaths) -> AllowanceState {
    std::fs::read(paths.usage())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_allowance(paths: &AppPaths, a: &AllowanceState) {
    if let Ok(b) = serde_json::to_vec(a) {
        if let Err(e) = write_atomic(&paths.usage(), &b) {
            log::warn!("could not write usage.json: {e}");
        }
    }
}

/// Start a job on its own thread. Fails when one is already running.
pub fn start(app: &AppHandle, job_id: String, req: PlanRequest, pro: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let mut slot = state.job.lock().unwrap();
        if slot.is_some() {
            return Err("Smidge is already working on a job. Wait for it to finish.".into());
        }
        *slot = Some(RunningJob {
            job_id: job_id.clone(),
            cancel: CancelToken::new(),
        });
    }
    let cancel = state
        .job
        .lock()
        .unwrap()
        .as_ref()
        .map(|j| j.cancel.clone())
        .unwrap_or_default();
    let engine = state.engine();
    let app = app.clone();
    std::thread::Builder::new()
        .name(format!("job-{job_id}"))
        .spawn(move || {
            let sink = TauriSink {
                app: app.clone(),
                pro,
                last_progress: Mutex::new(HashMap::new()),
            };
            let started = Instant::now();
            let (summary, log) = engine.run_job(&job_id, &req, &sink, &cancel);
            log::info!(
                "job {job_id}: {:?} in {} ms ({} → {} bytes)",
                summary.verdict,
                started.elapsed().as_millis(),
                summary.input_bytes,
                summary.total_bytes
            );
            finish(&app, &summary, &log.to_json());
        })
        .map_err(|e| format!("could not start the job thread: {e}"))?;
    Ok(())
}

fn finish(app: &AppHandle, summary: &JobSummary, log_json: &str) {
    let state = app.state::<AppState>();
    let mut dirs = Vec::new();
    {
        let mut artifacts = state.artifacts.lock().unwrap();
        let mut remember = |a: &Artifact| {
            let (OutputLocation::Path { path } | OutputLocation::Opfs { path }) = &a.location;
            let p = PathBuf::from(path);
            if let Some(dir) = p.parent() {
                dirs.push(dir.to_string_lossy().to_string());
            }
            artifacts.insert(a.id.clone(), p);
        };
        for (_, outcome) in &summary.outcomes {
            if let ItemOutcome::Fitted { artifact } | ItemOutcome::KeptOriginal { artifact } =
                outcome
            {
                remember(artifact);
            }
        }
        if let Some(p) = &summary.packaged {
            remember(p);
        }
    }
    dirs.sort();
    dirs.dedup();
    state.with_desktop(|d| {
        for dir in &dirs {
            d.remember_output_dir(dir);
        }
    });
    write_job_log(&state.paths, &summary.job_id, log_json);
    *state.job.lock().unwrap() = None;
}

/// `<app_data>/logs/jobs/<yyyymmdd-hhmmss>-<job_id>.json`, newest 200 kept (3.12).
pub fn write_job_log(paths: &AppPaths, job_id: &str, json: &str) {
    let dir = paths.jobs_dir();
    let name = format!("{}-{}.json", stamp(crate::paths::now_secs()), job_id);
    if let Err(e) = write_atomic(&dir.join(name), json.as_bytes()) {
        log::warn!("could not write job log: {e}");
    }
    for old in job_logs(paths).into_iter().skip(JOB_LOGS_KEPT) {
        let _ = std::fs::remove_file(old);
    }
}

/// Job log files, newest first (the name prefix sorts chronologically).
pub fn job_logs(paths: &AppPaths) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(paths.jobs_dir())
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

/// Sweep `.smidge-*.partial` files older than an hour in the folders recent jobs wrote to (3.10).
pub fn clean_partials(state: &AppState) {
    let dirs = state.desktop.lock().unwrap().recent_output_dirs.clone();
    for d in dirs {
        FsSink::clean_partials(Path::new(&d));
    }
    FsSink::clean_partials(&state.paths.temp_dir());
}
