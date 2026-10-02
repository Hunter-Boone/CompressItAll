//! `<app_data>/settings.json` (DESIGN.md 4.9). The UI owns the schema
//! (`packages/ui/src/lib/settings.ts`); this side only persists the blob,
//! forces `"v": 1`, and reads the few keys the Rust side needs.

use crate::paths::{write_atomic, AppPaths};
use serde_json::{Map, Value};

pub const VERSION: u64 = 1;

pub fn load(paths: &AppPaths) -> Map<String, Value> {
    let mut map = std::fs::read(paths.settings())
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    map.insert("v".into(), Value::from(VERSION));
    map
}

pub fn save(paths: &AppPaths, mut settings: Map<String, Value>) -> std::io::Result<()> {
    settings.insert("v".into(), Value::from(VERSION));
    let bytes = serde_json::to_vec_pretty(&Value::Object(settings))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&paths.settings(), &bytes)
}

/// Read one string key, or `None` when missing or not a string.
pub fn get_str(paths: &AppPaths, key: &str) -> Option<String> {
    load(paths)
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Set or remove one key.
pub fn set(paths: &AppPaths, key: &str, value: Option<Value>) -> std::io::Result<()> {
    let mut map = load(paths);
    match value {
        Some(v) => {
            map.insert(key.into(), v);
        }
        None => {
            map.remove(key);
        }
    }
    save(paths, map)
}

/// The updater channel from settings (`updateChannel`), "stable" unless the user chose "beta".
pub fn update_channel(paths: &AppPaths) -> String {
    match get_str(paths, "updateChannel").as_deref() {
        Some("beta") => "beta".into(),
        _ => "stable".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_forces_version() {
        let dir = tempdir();
        let paths = AppPaths::new(dir.clone());
        let mut m = Map::new();
        m.insert("theme".into(), Value::from("dark"));
        m.insert("v".into(), Value::from(99));
        save(&paths, m).unwrap();
        let back = load(&paths);
        assert_eq!(back["v"], Value::from(1));
        assert_eq!(back["theme"], Value::from("dark"));
        set(&paths, "ffmpegPath", Some(Value::from("/x/ffmpeg"))).unwrap();
        assert_eq!(get_str(&paths, "ffmpegPath").as_deref(), Some("/x/ffmpeg"));
        set(&paths, "ffmpegPath", None).unwrap();
        assert_eq!(get_str(&paths, "ffmpegPath"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tempdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("smidge-settings-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
