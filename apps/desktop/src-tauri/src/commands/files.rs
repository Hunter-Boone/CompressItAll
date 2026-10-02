//! File helpers the webview cannot do itself: the drag-out icon.

use crate::state::AppState;
use tauri::State;

/// The PNG under the cursor during drag-out, written once to `<app_data>/drag-icon.png`.
#[tauri::command]
pub fn drag_icon_path(state: State<'_, AppState>) -> Result<String, String> {
    let p = state.paths.drag_icon();
    if !p.exists() {
        std::fs::write(&p, include_bytes!("../../icons/128x128.png")).map_err(|e| e.to_string())?;
    }
    Ok(p.to_string_lossy().to_string())
}
