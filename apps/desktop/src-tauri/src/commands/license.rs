//! License commands (5.5, 5.6). Network calls run off the async runtime.

use super::blocking;
use crate::license::{self, LicenseInfo, PurchaseStart};
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn license_status(state: State<'_, AppState>) -> LicenseInfo {
    license::status(&state)
}

#[tauri::command]
pub async fn license_activate(app: AppHandle, product_key: String) -> Result<LicenseInfo, String> {
    blocking(move || license::activate(&app.state::<AppState>(), &product_key)).await
}

#[tauri::command]
pub async fn license_deactivate(app: AppHandle) -> Result<(), String> {
    blocking(move || license::deactivate(&app.state::<AppState>())).await
}

#[tauri::command]
pub async fn license_resend(email: String) -> Result<(), String> {
    blocking(move || license::resend(&email)).await
}

#[tauri::command]
pub async fn license_start_purchase(app: AppHandle, plan: String) -> Result<PurchaseStart, String> {
    if plan != "lifetime" && plan != "yearly" {
        return Err("unknown plan".into());
    }
    blocking(move || license::start_purchase(&app.state::<AppState>(), &plan)).await
}

#[tauri::command]
pub async fn license_poll_claim(app: AppHandle, claim_id: String) -> Result<&'static str, String> {
    blocking(move || license::poll_claim(&app.state::<AppState>(), &claim_id)).await
}
