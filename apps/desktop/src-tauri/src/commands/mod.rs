//! Tauri commands, one module per concern. Every command returns
//! `Result<T, String>`; the string is already in the words the UI shows.

pub mod clipboard;
pub mod diagnostics;
pub mod engine;
pub mod ffmpeg;
pub mod files;
pub mod license;
pub mod settings;
pub mod updates;

/// Run blocking work off the async runtime and flatten the join error.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("background task failed: {e}"))?
}
