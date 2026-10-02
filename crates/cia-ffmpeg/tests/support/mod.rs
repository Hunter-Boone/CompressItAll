//! Shared helpers for the integration tests: find an FFmpeg (never PATH in
//! the library; the tests themselves may fall back to the system binary) and
//! the synthetic fixtures.

#![allow(dead_code)]

use cia_ffmpeg::{use_own_copy, InstalledFfmpeg};
use std::path::{Path, PathBuf};

/// `CIA_FFMPEG`, else the system ffmpeg (tests only).
pub fn ffmpeg() -> Option<InstalledFfmpeg> {
    if let Some(p) = std::env::var_os("CIA_FFMPEG").filter(|v| !v.is_empty()) {
        return use_own_copy(Path::new(&p)).ok();
    }
    let candidates: &[&str] = if cfg!(windows) {
        &["C:\\ffmpeg\\bin\\ffmpeg.exe"]
    } else {
        &[
            "/usr/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
            "/opt/homebrew/bin/ffmpeg",
        ]
    };
    candidates
        .iter()
        .map(Path::new)
        .filter(|p| p.is_file())
        .find_map(|p| use_own_copy(p).ok())
}

/// `CIA_FIXTURES`, else `<workspace>/fixtures/synth`, when it has the video fixtures.
pub fn fixtures() -> Option<PathBuf> {
    let dir = std::env::var_os("CIA_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("fixtures")
                .join("synth")
        });
    dir.join("v_720p_30fps_10s.mp4").is_file().then_some(dir)
}

pub fn fixture(name: &str) -> Option<PathBuf> {
    fixtures().map(|d| d.join(name)).filter(|p| p.is_file())
}

/// Print why a test did nothing, so a green run without FFmpeg is visibly hollow.
pub fn skip(what: &str) {
    eprintln!("SKIPPED: {what}");
}

#[macro_export]
macro_rules! need {
    ($opt:expr, $why:literal) => {
        match $opt {
            Some(v) => v,
            None => {
                $crate::support::skip($why);
                return;
            }
        }
    };
}
