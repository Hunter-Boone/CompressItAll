//! "Copy file" (4.3): the result goes on the clipboard as a file list, not text
//! (Windows CF_HDROP, macOS file URLs on NSPasteboard, Linux `text/uri-list`
//! plus `x-special/gnome-copied-files`), through clipboard-rs. One context is
//! kept for the process: on X11 it owns the selection and serves paste requests
//! from a background thread for as long as it lives.

use crate::state::AppState;
use clipboard_rs::{Clipboard, ClipboardContext};
use tauri::State;

#[tauri::command]
pub fn copy_files(state: State<'_, AppState>, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut slot = state.clipboard.lock().unwrap();
    if slot.is_none() {
        *slot = Some(ClipboardContext::new().map_err(|e| {
            log::warn!("clipboard: {e}");
            "Smidge couldn't reach the clipboard. Use Show in folder and copy the file from there."
                .to_string()
        })?);
    }
    let ctx = slot.as_ref().expect("set above");
    ctx.set_files(paths).map_err(|e| {
        log::warn!("clipboard set_files: {e}");
        "Smidge couldn't put the file on the clipboard. Use Show in folder and copy it from there."
            .to_string()
    })
}
