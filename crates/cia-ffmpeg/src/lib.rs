//! Smidge native FFmpeg integration (DESIGN.md 3.5, 3.11, 4.8, 6.4).
//!
//! FFmpeg is never linked; it runs as a child process from the copy Smidge
//! downloaded into `<app_data>/tools/ffmpeg/<version>/` ([`install`]) or
//! from a copy the user pointed at ([`use_own_copy`]). `PATH` is never
//! searched, except through the explicit `CIA_FFMPEG` override that tests
//! and CI use.
//!
//! Modules, in the order a video flows through them:
//!
//! - [`probe`]: two `ffprobe` calls into a [`cia_video_plan::VideoProbe`].
//! - [`encoders`]: `ffmpeg -encoders` plus a one-second test encode per
//!   candidate, cached next to the binary, into [`cia_core::FfmpegCapabilities`].
//! - [`command`]: pure argument builders for the exact commands in 3.5.7.
//! - [`runner`]: spawn with `-progress pipe:1`, parse `out_time_us`, 60 s
//!   watchdog, process-tree cancel, partial-file cleanup.
//! - [`transcode`]: the retry loop of 3.5.8 over [`cia_video_plan`], the
//!   verification row of 3.11, frame extraction and HEIC/AVIF decode.
//! - [`install`] and [`pinned`]: signed manifest, SHA-256 checked zip, 4.8 errors.
//!
//! Paths are passed to the child as arguments, never through a shell.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod cancel;
pub mod command;
pub mod encoders;
pub mod install;
pub mod pinned;
pub mod probe;
pub mod process;
pub mod runner;
pub mod transcode;

use std::path::{Path, PathBuf};

pub use cancel::Cancel;
pub use command::{Encoder, Pass};
pub use encoders::{EncoderReport, GpuVendor};
#[cfg(feature = "download")]
pub use install::install;
pub use install::{
    locate, locate_with_override, remove, use_own_copy, InstallError, InstallProgress, Manifest,
    ManifestAsset,
};
pub use probe::probe;
pub use runner::{ProgressParser, ProgressUpdate, RunOptions, RunOutcome, STALL_TIMEOUT};
pub use transcode::{
    decode_image, extract_frame, verify_output, CompressRequest, ExpectedOutput, Session,
    VideoOutcome, VideoResult,
};

/// A usable FFmpeg: where `ffmpeg` and `ffprobe` are and what `-version` said.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstalledFfmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// First token after "ffmpeg version " in `ffmpeg -version`, e.g. "7.1.2" or "n7.1.2-3-g...".
    pub version: String,
    /// True for "Use my own copy" and the `CIA_FFMPEG` override; false for a Smidge install.
    pub user_supplied: bool,
    /// `<app_data>/tools/ffmpeg/<version>` for a Smidge install, None otherwise.
    pub install_dir: Option<PathBuf>,
}

impl InstalledFfmpeg {
    /// Where the encoder test results for this binary are cached (DESIGN 3.5.6).
    /// A Smidge install keeps them in its own folder; a user copy gets a
    /// folder keyed by the binary's path and version so two copies never share.
    pub fn encoders_cache_path(&self, app_data: &Path) -> PathBuf {
        if let Some(dir) = &self.install_dir {
            return dir.join("encoders.json");
        }
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(self.ffmpeg.to_string_lossy().as_bytes());
        h.update(b"\0");
        h.update(self.version.as_bytes());
        let digest = h.finalize();
        let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        install::tools_dir(app_data)
            .join("external")
            .join(key)
            .join("encoders.json")
    }
}

/// Everything that can go wrong while running FFmpeg. [`FfmpegError::failure_code`]
/// maps each case to the stable code `cia_core::copy::failure_message` knows.
#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error("ffprobe could not read the file: {detail}")]
    DamagedInput { detail: String },
    #[error("the video encoder stopped responding")]
    Stalled,
    #[error("cancelled")]
    Cancelled,
    #[error("ffmpeg exited with status {status}: {stderr}")]
    Exit { status: i32, stderr: String },
    #[error("ffmpeg could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("not enough free space")]
    DiskFull,
    #[error("ffmpeg produced output that could not be parsed: {0}")]
    BadOutput(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl FfmpegError {
    /// Stable failure code for `cia_core::Failure.code` / `copy::failure_message`.
    pub fn failure_code(&self) -> &'static str {
        match self {
            FfmpegError::DamagedInput { .. } => "damaged_video",
            FfmpegError::Stalled => "encoder_stalled",
            FfmpegError::Cancelled => "cancelled",
            FfmpegError::Exit { .. } => "encoder_crash",
            FfmpegError::Spawn(_) => "ffmpeg_missing",
            FfmpegError::DiskFull => "disk_full",
            FfmpegError::BadOutput(_) => "encoder_crash",
            FfmpegError::Io(e) if is_disk_full(e) => "disk_full",
            FfmpegError::Io(_) => "not_writable",
        }
    }

    /// Sentence for the user, from `cia_core::copy`.
    pub fn user_message(&self) -> String {
        cia_core::copy::failure_message(self.failure_code(), None, None)
    }
}

/// ENOSPC / ERROR_DISK_FULL / ERROR_HANDLE_DISK_FULL, or the portable kind.
pub fn is_disk_full(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::StorageFull {
        return true;
    }
    match e.raw_os_error() {
        #[cfg(unix)]
        Some(code) => code == 28,
        #[cfg(windows)]
        Some(code) => code == 112 || code == 39,
        #[cfg(not(any(unix, windows)))]
        Some(_) => false,
        None => false,
    }
}

/// `ffmpeg` / `ffprobe` with the platform's executable suffix.
pub fn exe_name(base: &str) -> String {
    if cfg!(windows) {
        format!("{base}.exe")
    } else {
        base.to_string()
    }
}

/// The version token from `ffmpeg -version` output ("ffmpeg version 7.1.2 Copyright ..." -> "7.1.2").
pub fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().next()?;
    let rest = line.strip_prefix("ffmpeg version ")?;
    let token = rest.split_whitespace().next()?;
    Some(token.trim_start_matches('n').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token() {
        assert_eq!(
            parse_version("ffmpeg version 5.1.9-0+deb12u1 Copyright (c) 2000-2026\nbuilt with gcc")
                .as_deref(),
            Some("5.1.9-0+deb12u1")
        );
        assert_eq!(
            parse_version("ffmpeg version n7.1.2 Copyright").as_deref(),
            Some("7.1.2")
        );
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn failure_codes_have_copy() {
        for e in [
            FfmpegError::DamagedInput {
                detail: String::new(),
            },
            FfmpegError::Stalled,
            FfmpegError::DiskFull,
        ] {
            let msg = e.user_message();
            assert!(!msg.contains("Something went wrong"), "{:?} -> {msg}", e);
        }
    }
}
