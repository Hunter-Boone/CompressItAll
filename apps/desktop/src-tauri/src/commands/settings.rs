//! Settings (4.9) and the free allowance (5.2).

use crate::state::AppState;
use serde_json::{Map, Value};
use tauri::State;

#[tauri::command]
pub fn settings_load(state: State<'_, AppState>) -> Map<String, Value> {
    crate::settings::load(&state.paths)
}

#[tauri::command]
pub fn settings_save(
    state: State<'_, AppState>,
    settings: Map<String, Value>,
) -> Result<(), String> {
    crate::settings::save(&state.paths, settings)
        .map_err(|e| format!("could not save settings: {e}"))
}

#[tauri::command]
pub fn allowance(state: State<'_, AppState>) -> Value {
    let mut a = crate::engine::load_allowance(&state.paths);
    let view = a.view(crate::paths::now_ms());
    crate::engine::save_allowance(&state.paths, &a);
    serde_json::json!({
        "remaining": view.remaining,
        "nextFreeAtMs": view.next_free_at_ms,
    })
}
