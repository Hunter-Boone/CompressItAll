//! Smidge desktop (DESIGN.md M3): the engine in-process behind a Tauri 2 window.
//! `run()` wires plugins, state and commands; the real work lives in the modules.

mod commands;
mod engine;
mod license;
mod paths;
mod settings;
mod state;

use paths::AppPaths;
use serde::{Deserialize, Serialize};
use state::{AppState, DesktopState};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use tauri::{Manager, WindowEvent};

/// `packages/brand/brand.json`, embedded so names and URLs come from one place.
pub fn brand() -> &'static serde_json::Value {
    static BRAND: OnceLock<serde_json::Value> = OnceLock::new();
    BRAND.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../packages/brand/brand.json"))
            .expect("brand.json is valid JSON")
    })
}

/// Remembered window geometry (4.2), `<app_data>/window.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowState {
    width: u32,
    height: u32,
    x: Option<i32>,
    y: Option<i32>,
    maximized: bool,
}

fn restore_window(window: &tauri::WebviewWindow, paths: &AppPaths) {
    let Some(ws) = std::fs::read(paths.window())
        .ok()
        .and_then(|b| serde_json::from_slice::<WindowState>(&b).ok())
    else {
        return;
    };
    if ws.width >= 760 && ws.height >= 560 {
        let _ = window.set_size(tauri::LogicalSize::new(ws.width, ws.height));
    }
    if let (Some(x), Some(y)) = (ws.x, ws.y) {
        let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    }
    if ws.maximized {
        let _ = window.maximize();
    }
}

fn save_window(window: &tauri::Window, paths: &AppPaths) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let maximized = window.is_maximized().unwrap_or(false);
    let size = window.inner_size().map(|s| s.to_logical::<u32>(scale)).ok();
    let pos = window
        .outer_position()
        .map(|p| p.to_logical::<i32>(scale))
        .ok();
    let Some(size) = size else { return };
    let ws = WindowState {
        width: size.width,
        height: size.height,
        x: pos.map(|p| p.x),
        y: pos.map(|p| p.y),
        maximized,
    };
    if let Ok(b) = serde_json::to_vec(&ws) {
        let _ = paths::write_atomic(&paths.window(), &b);
    }
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(feature = "drag")]
    let builder = builder.plugin(tauri_plugin_drag::init());

    builder
        .setup(|app| {
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("no app data dir: {e}"))?;
            let paths = AppPaths::new(app_data);
            paths.ensure_dirs()?;

            // App log: <app_data>/logs/app.log, 5 MB, 3 rotations (3.12). Info for
            // everything, Debug for our own crates in dev builds; codec crates stay quiet.
            let own = if cfg!(any(debug_assertions, feature = "dev-build")) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            };
            // `Builder::new()` starts with Stdout + LogDir + Webview targets; replace them.
            let mut log_builder = tauri_plugin_log::Builder::new()
                .clear_targets()
                .level(log::LevelFilter::Info)
                .level_for("cia_desktop", own)
                .level_for("cia_engine", own)
                .level_for("cia_image", own)
                .level_for("cia_ffmpeg", own)
                .level_for("oxipng", log::LevelFilter::Warn)
                .level_for("hyper", log::LevelFilter::Warn)
                .level_for("reqwest", log::LevelFilter::Warn)
                .level_for("rustls", log::LevelFilter::Warn)
                .level_for("zbus", log::LevelFilter::Warn)
                .level_for("tracing", log::LevelFilter::Warn)
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Folder {
                        path: paths.logs_dir(),
                        file_name: Some("app".into()),
                    },
                ));
            if cfg!(any(debug_assertions, feature = "dev-build")) {
                log_builder = log_builder.target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ));
            }
            app.handle().plugin(log_builder.build())?;
            log::info!(
                "Smidge {} starting; app data {}",
                env!("CARGO_PKG_VERSION"),
                paths.app_data.display()
            );

            let desktop = DesktopState::load(&paths);
            let ffmpeg = engine::locate_ffmpeg(&paths);
            let eng = engine::build(&paths, ffmpeg.as_ref());
            match &eng.caps.ffmpeg {
                Some(f) => log::info!("video support: FFmpeg {} ({})", f.version, f.path),
                None => log::info!("video support: not set up"),
            }
            app.manage(AppState {
                paths: paths.clone(),
                engine: RwLock::new(Arc::new(eng)),
                ffmpeg: RwLock::new(ffmpeg),
                desktop: Mutex::new(desktop),
                items: Default::default(),
                artifacts: Default::default(),
                job: Default::default(),
                preview_cancel: Default::default(),
                ffmpeg_cancel: Default::default(),
                integrity_failures: Default::default(),
                clipboard: Default::default(),
                claims: Default::default(),
                pending_update: Default::default(),
            });

            engine::clean_partials(&app.state::<AppState>());
            if let Some(w) = app.get_webview_window("main") {
                restore_window(&w, &paths);
            }
            license::spawn_refresh_loop(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { .. } = event {
                let paths = window.state::<AppState>().paths.clone();
                save_window(window, &paths);
            }
        })
        .invoke_handler(handlers())
        .run(tauri::generate_context!())
        .expect("error while running Smidge");
}

#[cfg(not(feature = "dev-build"))]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::engine::capabilities,
        commands::engine::add_paths,
        commands::engine::remove_input,
        commands::engine::preview,
        commands::engine::run,
        commands::engine::cancel,
        commands::engine::artifact_paths,
        commands::engine::preview_frame,
        commands::engine::version,
        commands::files::drag_icon_path,
        commands::clipboard::copy_files,
        commands::settings::settings_load,
        commands::settings::settings_save,
        commands::settings::allowance,
        commands::license::license_status,
        commands::license::license_activate,
        commands::license::license_deactivate,
        commands::license::license_resend,
        commands::license::license_start_purchase,
        commands::license::license_poll_claim,
        commands::ffmpeg::ffmpeg_manifest,
        commands::ffmpeg::ffmpeg_install,
        commands::ffmpeg::ffmpeg_use_own_copy,
        commands::ffmpeg::ffmpeg_remove,
        commands::ffmpeg::ffmpeg_retest,
        commands::ffmpeg::ffmpeg_cancel,
        commands::updates::updates_check,
        commands::updates::updates_install,
        commands::diagnostics::diagnostics_report,
        commands::diagnostics::diagnostics_open_logs,
        commands::diagnostics::diagnostics_job_log,
    ]
}

#[cfg(feature = "dev-build")]
fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        commands::engine::capabilities,
        commands::engine::add_paths,
        commands::engine::remove_input,
        commands::engine::preview,
        commands::engine::run,
        commands::engine::cancel,
        commands::engine::artifact_paths,
        commands::engine::preview_frame,
        commands::engine::version,
        commands::engine::debug_add_paths,
        commands::files::drag_icon_path,
        commands::clipboard::copy_files,
        commands::settings::settings_load,
        commands::settings::settings_save,
        commands::settings::allowance,
        commands::license::license_status,
        commands::license::license_activate,
        commands::license::license_deactivate,
        commands::license::license_resend,
        commands::license::license_start_purchase,
        commands::license::license_poll_claim,
        commands::ffmpeg::ffmpeg_manifest,
        commands::ffmpeg::ffmpeg_install,
        commands::ffmpeg::ffmpeg_use_own_copy,
        commands::ffmpeg::ffmpeg_remove,
        commands::ffmpeg::ffmpeg_retest,
        commands::ffmpeg::ffmpeg_cancel,
        commands::updates::updates_check,
        commands::updates::updates_install,
        commands::diagnostics::diagnostics_report,
        commands::diagnostics::diagnostics_open_logs,
        commands::diagnostics::diagnostics_job_log,
    ]
}
