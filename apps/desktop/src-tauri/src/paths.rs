//! Every file the desktop app keeps under `<app_data>` (DESIGN.md 3.10, 3.12,
//! 4.8, 4.9, 5.5), in one place so nothing else builds paths by hand.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub app_data: PathBuf,
}

impl AppPaths {
    pub fn new(app_data: PathBuf) -> Self {
        Self { app_data }
    }

    /// `settings.json`, read and written only through `settings.rs`.
    pub fn settings(&self) -> PathBuf {
        self.app_data.join("settings.json")
    }
    /// The license token, plain text (5.5).
    pub fn license_token(&self) -> PathBuf {
        self.app_data.join("license.token")
    }
    /// `state.json`: license clock state, recent output folders, counters.
    pub fn state(&self) -> PathBuf {
        self.app_data.join("state.json")
    }
    /// Free-allowance uses (5.2).
    pub fn usage(&self) -> PathBuf {
        self.app_data.join("usage.json")
    }
    /// Remembered window size and position (4.2).
    pub fn window(&self) -> PathBuf {
        self.app_data.join("window.json")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.app_data.join("logs")
    }
    pub fn app_log(&self) -> PathBuf {
        self.logs_dir().join("app.log")
    }
    pub fn jobs_dir(&self) -> PathBuf {
        self.logs_dir().join("jobs")
    }
    pub fn diagnostics_dir(&self) -> PathBuf {
        self.app_data.join("diagnostics")
    }
    pub fn temp_dir(&self) -> PathBuf {
        self.app_data.join("tmp")
    }
    /// The PNG shown under the cursor during drag-out.
    pub fn drag_icon(&self) -> PathBuf {
        self.app_data.join("drag-icon.png")
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for d in [
            self.app_data.as_path(),
            &self.logs_dir(),
            &self.jobs_dir(),
            &self.temp_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }
}

/// Write `bytes` to `path` through a sibling temp file so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `yyyymmdd-hhmmss` in UTC for log file names (3.12).
pub fn stamp(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs = unix_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp(1_759_400_000), "20251002-101320");
    }
}
