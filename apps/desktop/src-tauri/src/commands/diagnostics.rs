//! Settings → About (3.12): the diagnostic ZIP, the logs folder, a job's log path.

use super::blocking;
use crate::state::AppState;
use std::io::Write;
use tauri::{AppHandle, Manager, State};

/// ZIP the last 10 job logs, app.log and the capabilities into
/// `<app_data>/diagnostics/`, then reveal it so the user can attach it to an email.
#[tauri::command]
pub async fn diagnostics_report(app: AppHandle) -> Result<(), String> {
    let path = blocking(move || {
        let state = app.state::<AppState>();
        let paths = &state.paths;
        std::fs::create_dir_all(paths.diagnostics_dir()).map_err(|e| e.to_string())?;
        let out = paths.diagnostics_dir().join(format!(
            "smidge-diagnostics-{}.zip",
            crate::paths::stamp(crate::paths::now_secs())
        ));
        let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let add = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, bytes: &[u8]| {
            zip.start_file(name, opts)
                .and_then(|_| zip.write_all(bytes).map_err(Into::into))
                .map_err(|e| e.to_string())
        };
        for log in crate::engine::job_logs(paths).into_iter().take(10) {
            if let (Some(name), Ok(bytes)) = (log.file_name(), std::fs::read(&log)) {
                add(
                    &mut zip,
                    &format!("jobs/{}", name.to_string_lossy()),
                    &bytes,
                )?;
            }
        }
        if let Ok(bytes) = std::fs::read(paths.app_log()) {
            add(&mut zip, "app.log", &bytes)?;
        }
        let caps = serde_json::to_vec_pretty(&state.engine().caps).unwrap_or_default();
        add(&mut zip, "capabilities.json", &caps)?;
        let info = serde_json::json!({
            "app": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "ffmpeg": *state.ffmpeg.read().unwrap(),
            "settings": crate::settings::load(paths),
        });
        add(
            &mut zip,
            "info.json",
            &serde_json::to_vec_pretty(&info).unwrap_or_default(),
        )?;
        zip.finish().map_err(|e| e.to_string())?;
        Ok(out)
    })
    .await?;
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn diagnostics_open_logs(state: State<'_, AppState>) -> Result<(), String> {
    let dir = state.paths.logs_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    tauri_plugin_opener::open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Path of the job's log file, or None when it was not written.
#[tauri::command]
pub fn diagnostics_job_log(state: State<'_, AppState>, job_id: String) -> Option<String> {
    let suffix = format!("-{job_id}.json");
    crate::engine::job_logs(&state.paths)
        .into_iter()
        .find(|p| p.to_string_lossy().ends_with(&suffix))
        .map(|p| p.to_string_lossy().to_string())
}
