//! Desktop licensing (DESIGN.md 5.5, 5.6): the token file, offline validation
//! on every call, the site API over reqwest/rustls, and the background refresh
//! on launch and every 24 hours.

use crate::paths::{now_secs, write_atomic, AppPaths};
use crate::state::AppState;
use cia_license::{LicenseStatus, Plan};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::Manager;

/// Compile-time override for the site API (e2e points it at the mock server), else brand.json.
pub fn api_base() -> String {
    if let Some(b) = option_env!("CIA_API_BASE").filter(|s| !s.is_empty()) {
        return b.trim_end_matches('/').to_string();
    }
    crate::brand()["urls"]["api"]
        .as_str()
        .unwrap_or("https://www.smidge.example/api/v1")
        .trim_end_matches('/')
        .to_string()
}

pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    }
}

/// What the UI shows (`LicenseInfo` in packages/engine-client).
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key4: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_masked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices_used: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices_max: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_until: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub fn read_token(paths: &AppPaths) -> Option<String> {
    std::fs::read_to_string(paths.license_token())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn write_token(paths: &AppPaths, token: &str) -> Result<(), String> {
    write_atomic(&paths.license_token(), token.as_bytes())
        .map_err(|e| format!("could not save the license: {e}"))
}

pub fn delete_token(paths: &AppPaths) {
    let _ = std::fs::remove_file(paths.license_token());
}

fn plan_str(p: Plan) -> &'static str {
    match p {
        Plan::Lifetime => "lifetime",
        Plan::Yearly => "yearly",
    }
}

/// Offline validation, run on every `license()` call (5.5). Falls back to the
/// free tier for anything but `Valid`; a one-time refresh notice rides along.
pub fn status(state: &AppState) -> LicenseInfo {
    status_with(state, true)
}

/// `status` without consuming the one-time refresh notice (for internal checks).
pub fn is_pro(state: &AppState) -> bool {
    status_with(state, false).status == "pro"
}

fn status_with(state: &AppState, take_notice: bool) -> LicenseInfo {
    let notice = state.with_desktop(|d| {
        if take_notice {
            d.license_notice.take()
        } else {
            None
        }
    });
    let Some(token) = read_token(&state.paths) else {
        return LicenseInfo {
            status: "free",
            message: notice,
            ..Default::default()
        };
    };
    let device = cia_license::device::device_hash();
    let now = now_secs();
    let (verdict, key_masked, used, max) = state.with_desktop(|d| {
        let v = cia_license::validate(&token, &device, now, &mut d.license);
        (v, d.key_masked.clone(), d.devices_used, d.devices_max)
    });
    match verdict {
        LicenseStatus::Valid { plan, key4, acc } => LicenseInfo {
            status: "pro",
            plan: Some(plan_str(plan)),
            key4: Some(key4),
            key_masked,
            devices_used: used,
            devices_max: max,
            access_until: Some(acc.map(|s| s * 1000)),
            message: notice,
        },
        LicenseStatus::Invalid { reason } => LicenseInfo {
            status: "invalid",
            message: Some(reason),
            ..Default::default()
        },
        LicenseStatus::NeedsOnline { reason } => LicenseInfo {
            status: "needs_online",
            message: Some(reason),
            ..Default::default()
        },
        LicenseStatus::Ended { at } => LicenseInfo {
            status: "ended",
            message: Some(format!(
                "Your yearly plan ended on {}.",
                crate::paths::stamp(at).split('-').next().unwrap_or("")
            )),
            ..Default::default()
        },
    }
}

// ---- HTTP -------------------------------------------------------------------

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("Smidge/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    devices: Vec<ApiDevice>,
}

#[derive(Debug, Deserialize)]
struct ApiDevice {
    name: Option<String>,
    #[serde(default)]
    last_seen_at: Option<String>,
}

pub const OFFLINE: &str =
    "Smidge couldn't reach the license server. Check your internet connection and try again.";

/// POST JSON; 2xx → body, else the API's message in 4.x wording.
fn post<T: for<'de> Deserialize<'de>>(path: &str, body: &serde_json::Value) -> Result<T, String> {
    let url = format!("{}{}", api_base(), path);
    let resp = client()?.post(&url).json(body).send().map_err(|e| {
        log::warn!("POST {url}: {e}");
        OFFLINE.to_string()
    })?;
    let status = resp.status();
    let text = resp.text().map_err(|_| OFFLINE.to_string())?;
    if status.is_success() {
        return serde_json::from_str(&text)
            .map_err(|e| format!("The license server sent an unexpected answer ({e})."));
    }
    Err(api_error_message(status.as_u16(), &text))
}

fn api_error_message(status: u16, text: &str) -> String {
    let Ok(err) = serde_json::from_str::<ApiError>(text) else {
        return match status {
            429 => "Too many attempts. Wait a minute and try again.".to_string(),
            500..=599 => {
                "The license server is having trouble. Try again in a few minutes.".to_string()
            }
            _ => format!("The license server answered {status}."),
        };
    };
    match err.error.as_str() {
        "key_not_found" => "That key wasn't found. Check the email we sent you.".to_string(),
        "device_limit" => {
            let names: Vec<String> = err
                .devices
                .iter()
                .map(|d| {
                    let name = d.name.clone().unwrap_or_else(|| "Unnamed computer".into());
                    match &d.last_seen_at {
                        Some(at) => format!("{name} (last used {})", ago(at)),
                        None => name,
                    }
                })
                .collect();
            format!(
                "This key is already used on {} computers: {}. Remove one in your account, then try again.",
                err.devices.len(),
                names.join(", ")
            )
        }
        "revoked" => "This key was refunded or disputed, so it no longer works.".to_string(),
        "ended" => "This yearly plan has ended. Renew it in your account to keep using Smidge Pro."
            .to_string(),
        _ if !err.message.is_empty() => err.message,
        other => format!("The license server answered {other}."),
    }
}

/// "3 days ago" from an RFC 3339 date; the raw date when it cannot be parsed.
fn ago(iso: &str) -> String {
    let Some(secs) = parse_rfc3339(iso) else {
        return iso.to_string();
    };
    let d = (now_secs() - secs).max(0);
    match d {
        0..=3599 => "just now".to_string(),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => {
            let days = d / 86_400;
            if days == 1 {
                "1 day ago".to_string()
            } else {
                format!("{days} days ago")
            }
        }
    }
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    // yyyy-mm-ddThh:mm:ss(.fff)?(Z|+00:00)
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hh, mm, ss) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let days = days_from_civil(y, m, d);
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn device_body() -> serde_json::Value {
    serde_json::json!({
        "kind": "desktop",
        "device_hash": cia_license::device::device_hash(),
        "device_name": cia_license::device::device_name(),
        "platform": platform(),
    })
}

#[derive(Debug, Deserialize)]
struct ActivateResponse {
    token: String,
    devices_used: u32,
    devices_max: u32,
}

/// 5.6.2. The key is checked locally first, so a typo never reaches the network.
pub fn activate(state: &AppState, product_key: &str) -> Result<LicenseInfo, String> {
    let key = cia_license::key::normalise(product_key)
        .map_err(|_| "That key has a typo. Check the email we sent you.".to_string())?;
    let mut body = device_body();
    body["product_key"] = serde_json::Value::from(key.clone());
    body["app_version"] = serde_json::Value::from(env!("CARGO_PKG_VERSION"));
    let r: ActivateResponse = post("/license/activate", &body)?;
    write_token(&state.paths, &r.token)?;
    state.with_desktop(|d| {
        d.key_masked = Some(cia_license::key::masked(&key));
        d.devices_used = Some(r.devices_used);
        d.devices_max = Some(r.devices_max);
        d.license.last_refresh_utc = Some(now_secs());
    });
    Ok(status(state))
}

/// Free this computer's slot on the server, then forget the token.
pub fn deactivate(state: &AppState) -> Result<(), String> {
    let Some(token) = read_token(&state.paths) else {
        return Ok(());
    };
    let body = serde_json::json!({ "token": token });
    match post::<serde_json::Value>("/license/deactivate", &body) {
        Ok(_) => {}
        // A server that no longer knows the device has already freed it.
        Err(m) if m.contains("deactivated") || m.contains("not found") => {}
        Err(m) => return Err(m),
    }
    delete_token(&state.paths);
    state.with_desktop(|d| {
        d.key_masked = None;
        d.devices_used = None;
        d.devices_max = None;
    });
    Ok(())
}

pub fn resend(email: &str) -> Result<(), String> {
    let body = serde_json::json!({ "email": email.trim() });
    post::<serde_json::Value>("/license/resend", &body).map(|_| ())
}

#[derive(Debug, Deserialize)]
struct ClaimCreated {
    claim_id: String,
    claim_secret: String,
    buy_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurchaseStart {
    pub buy_url: String,
    pub claim_id: String,
}

/// 5.6.1: create a claim; the browser finishes the purchase and the app polls.
pub fn start_purchase(state: &AppState, plan: &str) -> Result<PurchaseStart, String> {
    let c: ClaimCreated = post("/claims", &device_body())?;
    state
        .claims
        .lock()
        .unwrap()
        .insert(c.claim_id.clone(), c.claim_secret);
    let sep = if c.buy_url.contains('?') { '&' } else { '?' };
    Ok(PurchaseStart {
        buy_url: format!("{}{sep}plan={plan}", c.buy_url),
        claim_id: c.claim_id,
    })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ClaimStatus {
    Pending,
    Fulfilled { product_key: String, token: String },
    Expired,
}

pub fn poll_claim(state: &AppState, claim_id: &str) -> Result<&'static str, String> {
    let secret = state
        .claims
        .lock()
        .unwrap()
        .get(claim_id)
        .cloned()
        .ok_or_else(|| "Unknown claim. Start the purchase again.".to_string())?;
    let url = format!("{}/claims/{claim_id}", api_base());
    let resp = client()?
        .get(&url)
        .header("Authorization", format!("Claim {secret}"))
        .send()
        .map_err(|_| OFFLINE.to_string())?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let text = resp.text().unwrap_or_default();
        return Err(api_error_message(code, &text));
    }
    let s: ClaimStatus = resp
        .json()
        .map_err(|e| format!("The license server sent an unexpected answer ({e})."))?;
    match s {
        ClaimStatus::Pending => Ok("pending"),
        ClaimStatus::Expired => Ok("expired"),
        ClaimStatus::Fulfilled { product_key, token } => {
            write_token(&state.paths, &token)?;
            state.with_desktop(|d| {
                d.key_masked = cia_license::key::normalise(&product_key)
                    .ok()
                    .map(|k| cia_license::key::masked(&k));
                d.devices_used = Some(1);
                d.devices_max = Some(3);
                d.license.last_refresh_utc = Some(now_secs());
            });
            state.claims.lock().unwrap().remove(claim_id);
            Ok("fulfilled")
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RefreshResponse {
    Ok { token: String },
    Revoked { reason: String },
    Ended {},
    Deactivated {},
}

/// 5.6.4. A network failure changes nothing; revoked / ended / deactivated
/// delete the token and leave a notice for the UI to show once.
pub fn refresh(state: &AppState) {
    let Some(token) = read_token(&state.paths) else {
        return;
    };
    let body = serde_json::json!({ "token": token });
    let r: RefreshResponse = match post("/license/refresh", &body) {
        Ok(r) => r,
        Err(e) => {
            log::info!("license refresh skipped: {e}");
            return;
        }
    };
    let notice = match r {
        RefreshResponse::Ok { token } => {
            if let Err(e) = write_token(&state.paths, &token) {
                log::warn!("{e}");
            }
            state.with_desktop(|d| d.license.last_refresh_utc = Some(now_secs()));
            return;
        }
        RefreshResponse::Revoked { reason } => match reason.as_str() {
            "chargeback" => "Your purchase was disputed with the card issuer, so Smidge Pro was turned off.",
            _ => "Your purchase was refunded, so Smidge Pro was turned off.",
        },
        RefreshResponse::Ended {} => "Your yearly plan has ended. Smidge is back on the free plan.",
        RefreshResponse::Deactivated {} => {
            "This computer was removed from your Smidge account, so Smidge is back on the free plan."
        }
    };
    delete_token(&state.paths);
    state.with_desktop(|d| {
        d.license_notice = Some(notice.to_string());
        d.key_masked = None;
        d.devices_used = None;
        d.devices_max = None;
    });
}

/// Refresh after the UI is up and then every 24 hours (5.5).
pub fn spawn_refresh_loop(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("license-refresh".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            loop {
                refresh(&app.state::<AppState>());
                std::thread::sleep(Duration::from_secs(24 * 60 * 60));
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_roundtrip() {
        assert_eq!(parse_rfc3339("2025-10-02T10:13:20Z"), Some(1_759_400_000));
        assert_eq!(
            parse_rfc3339("2025-10-02T10:13:20.123+00:00"),
            Some(1_759_400_000)
        );
        assert_eq!(parse_rfc3339("nope"), None);
    }

    #[test]
    fn device_limit_message() {
        let text = r#"{"error":"device_limit","message":"x","devices":[{"name":"Margaret's PC","platform":"windows","last_seen_at":"2000-01-01T00:00:00Z"},{"name":null}]}"#;
        let m = api_error_message(409, text);
        assert!(m.starts_with("This key is already used on 2 computers: Margaret's PC (last used"));
        assert!(m.ends_with("Unnamed computer. Remove one in your account, then try again."));
        assert_eq!(
            api_error_message(404, r#"{"error":"key_not_found","message":""}"#),
            "That key wasn't found. Check the email we sent you."
        );
    }
}
