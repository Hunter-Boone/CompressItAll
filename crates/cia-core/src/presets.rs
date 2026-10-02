//! Presets: `packages/presets/presets.json` is the single source of truth,
//! embedded at build time (DESIGN.md 3.13).

use crate::limits::{raw_budget, Counts};
use crate::model::{LimitKind, LimitScope, ResolvedKindLimit, ResolvedLimit};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub const PRESETS_JSON: &str = include_str!("../../../packages/presets/presets.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetFile {
    pub schema_version: u32,
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PresetMode {
    Fit,
    Smaller,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CountsAs {
    Raw,
    MimeBase64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetLimit {
    pub bytes: u64,
    pub stated_as: String,
    pub scope: LimitScope,
    pub counts: CountsAs,
    pub safety_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoFormat {
    pub container: String,
    pub video: String,
    pub audio: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetFormats {
    pub image: Vec<String>,
    pub animated: Vec<String>,
    pub video: Vec<VideoFormat>,
    pub audio: Vec<String>,
    pub document: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct VideoCaps {
    pub max_short_edge: Option<u32>,
    pub max_fps: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PresetSource {
    pub urls: Vec<String>,
    pub checked_on: String,
    pub confidence: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Preset {
    pub id: String,
    pub group: String,
    pub tile_label: String,
    #[serde(default)]
    pub tier_label: Option<String>,
    #[serde(default)]
    pub tier_summary: Option<String>,
    pub output_label: String,
    pub icon: String,
    pub order: i32,
    pub mode: PresetMode,
    #[serde(default)]
    pub default_tier: bool,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub limit: Option<PresetLimit>,
    #[serde(default)]
    #[ts(type = "Record<string, PresetLimit>")]
    pub limit_by_kind: BTreeMap<LimitKind, PresetLimit>,
    #[serde(default)]
    pub max_files_per_message: Option<u32>,
    pub formats: PresetFormats,
    #[serde(default)]
    pub video_caps: VideoCaps,
    #[serde(default)]
    pub source: Option<PresetSource>,
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("presets.json is not valid: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unknown preset id {0}")]
    Unknown(String),
    #[error("preset {0} has no byte limit; Custom needs a value")]
    NoLimit(String),
}

pub fn load() -> Result<PresetFile, PresetError> {
    Ok(serde_json::from_str(PRESETS_JSON)?)
}

/// Parsed once; cheap to clone.
pub fn all() -> Vec<Preset> {
    load()
        .expect("embedded presets.json is valid (checked by tests)")
        .presets
}

pub fn find(id: &str) -> Result<Preset, PresetError> {
    all()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| PresetError::Unknown(id.to_string()))
}

fn counts(c: CountsAs) -> Counts {
    match c {
        CountsAs::Raw => Counts::Raw,
        CountsAs::MimeBase64 => Counts::MimeBase64,
    }
}

impl Preset {
    /// Resolve a fit preset into byte numbers.
    pub fn resolve(&self) -> Result<ResolvedLimit, PresetError> {
        let limit = self
            .limit
            .as_ref()
            .ok_or_else(|| PresetError::NoLimit(self.id.clone()))?;
        let mut by_kind = BTreeMap::new();
        for (k, l) in &self.limit_by_kind {
            by_kind.insert(
                *k,
                ResolvedKindLimit {
                    hard_bytes: l.bytes,
                    raw_budget_bytes: raw_budget(l.bytes, counts(l.counts), l.safety_bytes),
                    safety_bytes: l.safety_bytes,
                },
            );
        }
        let raw = raw_budget(limit.bytes, counts(limit.counts), limit.safety_bytes);
        Ok(ResolvedLimit {
            hard_bytes: limit.bytes,
            raw_budget_bytes: raw,
            safety_bytes: limit.safety_bytes,
            scope: limit.scope,
            max_files_per_message: self.max_files_per_message,
            by_kind,
            output_label: self.output_label.clone(),
            stated_label: self.stated_label(),
        })
    }

    /// "20 MB", or "18 MB of attachments" for email-style limits.
    pub fn stated_label(&self) -> String {
        match &self.limit {
            Some(l) if l.counts == CountsAs::MimeBase64 => {
                format!(
                    "{} of attachments",
                    crate::format::mb_whole(raw_budget(
                        l.bytes,
                        Counts::MimeBase64,
                        l.safety_bytes
                    ))
                )
            }
            Some(l) => crate::format::mb_whole(l.bytes),
            None => String::new(),
        }
    }
}

/// Resolve the Custom preset for a user-entered limit.
pub fn resolve_custom(bytes: u64, per_message: bool) -> ResolvedLimit {
    let scope = if per_message {
        LimitScope::PerMessage
    } else {
        LimitScope::PerFile
    };
    ResolvedLimit {
        hard_bytes: bytes,
        raw_budget_bytes: bytes,
        safety_bytes: 0,
        scope,
        max_files_per_message: None,
        by_kind: BTreeMap::new(),
        output_label: format!("Custom {}", crate::format::mb_whole(bytes)),
        stated_label: crate::format::mb_whole(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_has_expected_ids() {
        let f = load().unwrap();
        assert_eq!(f.schema_version, 1);
        for id in [
            "discord-free",
            "email",
            "whatsapp",
            "imessage",
            "custom",
            "smaller",
        ] {
            assert!(f.presets.iter().any(|p| p.id == id), "{id}");
        }
        let mut ids: Vec<&str> = f.presets.iter().map(|p| p.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate preset ids");
    }

    #[test]
    fn fit_presets_resolve_to_design_numbers() {
        let d = find("discord-free").unwrap().resolve().unwrap();
        assert_eq!(d.hard_bytes, 20_971_520);
        assert_eq!(d.raw_budget_bytes, 20_971_520 - 65_536);
        assert_eq!(d.scope, LimitScope::PerFile);
        assert_eq!(d.max_files_per_message, Some(10));
        let e = find("email").unwrap().resolve().unwrap();
        assert_eq!(e.raw_budget_bytes, 18_196_153);
        assert_eq!(e.scope, LimitScope::PerMessage);
        assert_eq!(e.stated_label, "18 MB of attachments");
        let w = find("whatsapp").unwrap().resolve().unwrap();
        assert_eq!(w.by_kind[&LimitKind::Document].hard_bytes, 2_000_000_000);
        assert_eq!(w.hard_for(crate::Kind::Pdf), 2_000_000_000);
        assert_eq!(w.hard_for(crate::Kind::Video), 64_000_000);
    }

    #[test]
    fn every_fit_preset_has_limit_source_and_formats() {
        for p in all() {
            match p.mode {
                PresetMode::Fit => {
                    let l = p.limit.as_ref().expect("limit");
                    assert!(l.bytes > 0);
                    assert!(l.safety_bytes < l.bytes);
                    let s = p.source.as_ref().expect("source");
                    assert!(!s.urls.is_empty());
                    assert!(
                        ["official", "third_party", "community"].contains(&s.confidence.as_str())
                    );
                }
                _ => assert!(p.limit.is_none()),
            }
            assert!(!p.formats.image.is_empty());
            assert!(!p.formats.video.is_empty());
            assert!(!p.formats.audio.is_empty());
        }
    }

    #[test]
    fn sources_checked_within_a_year_of_build() {
        // Build date from the system clock; the design asks for a warning at 120 days and an error at 365.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        for p in all() {
            if let Some(s) = &p.source {
                let parts: Vec<i64> = s
                    .checked_on
                    .split('-')
                    .map(|x| x.parse().unwrap())
                    .collect();
                let checked = days_from_civil(parts[0], parts[1] as u32, parts[2] as u32) * 86_400;
                let age_days = (now - checked) / 86_400;
                assert!(
                    age_days < 365,
                    "preset {} source is {} days old: re-check the limit",
                    p.id,
                    age_days
                );
                if age_days > 120 {
                    eprintln!(
                        "warning: preset {} limit source is {} days old",
                        p.id, age_days
                    );
                }
            }
        }
    }

    fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (m as i64 + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    #[test]
    fn custom_label() {
        let c = resolve_custom(8_000_000, false);
        assert_eq!(c.output_label, "Custom 8 MB");
        assert_eq!(c.hard_bytes, 8_000_000);
    }
}
