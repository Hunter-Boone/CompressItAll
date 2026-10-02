//! Updater (6.3): built at runtime so the channel from settings picks the
//! endpoints; the install id rides along for the staged rollout.

use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// The two endpoints of 6.3 for a channel. The domain is the brand.json
/// placeholder until Hunter picks one.
pub fn endpoints(channel: &str) -> Vec<String> {
    let brand = crate::brand();
    let site = brand["urls"]["site"]
        .as_str()
        .unwrap_or("https://www.smidge.example")
        .trim_end_matches('/');
    let downloads = brand["urls"]["downloads_repo"]
        .as_str()
        .unwrap_or("https://github.com/Hunter-Boone/Smidge-Downloads")
        .trim_end_matches('/')
        .replace("https://github.com/", "https://raw.githubusercontent.com/");
    vec![
        format!(
            "{site}/api/v1/updates/{channel}/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}"
        ),
        format!("{downloads}/main/channels/{channel}.tauri.json"),
    ]
}

fn install_id(state: &AppState) -> String {
    state.with_desktop(|d| {
        d.install_id
            .get_or_insert_with(|| {
                let mut b = [0u8; 16];
                if getrandom::fill(&mut b).is_err() {
                    b.copy_from_slice(&crate::paths::now_ms().to_le_bytes().repeat(2));
                }
                b.iter().map(|x| format!("{x:02x}")).collect()
            })
            .clone()
    })
}

#[tauri::command]
pub async fn updates_check(app: AppHandle) -> Result<UpdateCheck, String> {
    let (channel, id) = {
        let state = app.state::<AppState>();
        (
            crate::settings::update_channel(&state.paths),
            install_id(&state),
        )
    };
    let urls = endpoints(&channel)
        .into_iter()
        .map(|u| u.parse::<url::Url>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let updater = app
        .updater_builder()
        .endpoints(urls)
        .map_err(|e| e.to_string())?
        .header("X-Smidge-Install", id)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => {
            let out = UpdateCheck {
                available: true,
                version: Some(update.version.clone()),
                notes: update.body.clone(),
            };
            *app.state::<AppState>().pending_update.lock().unwrap() = Some(update);
            Ok(out)
        }
        Ok(None) => Ok(UpdateCheck {
            available: false,
            version: None,
            notes: None,
        }),
        Err(e) => {
            // No network, no release yet, or a .deb that cannot self-update: the UI
            // says "You're up to date" and the log says why.
            log::info!("update check ({channel}): {e}");
            Ok(UpdateCheck {
                available: false,
                version: None,
                notes: None,
            })
        }
    }
}

/// Download, install and restart. Never during a job (6.3).
#[tauri::command]
pub async fn updates_install(app: AppHandle) -> Result<(), String> {
    let update = {
        let state = app.state::<AppState>();
        if state.job_running() {
            return Err("Smidge will update after the current job finishes.".into());
        }
        let pending = state.pending_update.lock().unwrap().clone();
        pending
    };
    let Some(update) = update else {
        return Err("Check for updates first.".into());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| {
            log::warn!("update install: {e}");
            "The update couldn't be installed. Download the new version from the Smidge site instead."
                .to_string()
        })?;
    app.restart();
}
