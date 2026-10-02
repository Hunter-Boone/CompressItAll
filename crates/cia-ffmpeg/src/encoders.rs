//! Which encoders this FFmpeg can really use (DESIGN.md 3.5.6).
//!
//! `ffmpeg -encoders` says what was compiled in; a listed hardware encoder
//! still fails when the driver or GPU is missing, so every candidate gets a
//! one-second `testsrc2` encode at first use. Results are cached next to the
//! binary (`encoders.json`) keyed by path and version. The GPU vendor falls
//! out of the same test: whichever of NVENC, AMF or QSV works.

use crate::cancel::Cancel;
use crate::command::{encoders_list_args, test_audio_encode_args, test_encode_args, Encoder};
use crate::process::run_capture;
use crate::{FfmpegError, InstalledFfmpeg};
use cia_core::FfmpegCapabilities;
use cia_video_plan::{encoder_chain, Gpu, Platform};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// A one-second test encode that takes longer than this is treated as broken.
pub const TEST_ENCODE_DEADLINE: Duration = Duration::from_secs(45);

/// Audio encoders worth proving, in the order they are tested.
pub const AUDIO_CANDIDATES: [&str; 3] = ["aac", "libopus", "libmp3lame"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
}

impl GpuVendor {
    pub fn to_plan(self) -> Gpu {
        match self {
            GpuVendor::Nvidia => Gpu::Nvidia,
            GpuVendor::Amd => Gpu::Amd,
            GpuVendor::Intel => Gpu::Intel,
        }
    }
}

/// What detection found; serialised as `encoders.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EncoderReport {
    pub schema: u32,
    pub ffmpeg_path: String,
    pub version: String,
    /// Every encoder name `-encoders` listed.
    pub listed: Vec<String>,
    /// Candidates whose test encode succeeded, video and audio.
    pub working: Vec<String>,
    /// Candidate -> tail of stderr for the ones that were listed but failed.
    pub failed: BTreeMap<String, String>,
    /// Candidates not listed, or not applicable on this platform.
    pub skipped: Vec<String>,
}

impl EncoderReport {
    pub const SCHEMA: u32 = 1;

    pub fn works(&self, name: &str) -> bool {
        self.working.iter().any(|w| w == name)
    }

    /// The vendor whose H.264 encoder passed, for `encoder_chain`.
    pub fn gpu(&self) -> Option<GpuVendor> {
        if self.works("h264_nvenc") {
            Some(GpuVendor::Nvidia)
        } else if self.works("h264_amf") {
            Some(GpuVendor::Amd)
        } else if self.works("h264_qsv") {
            Some(GpuVendor::Intel)
        } else {
            None
        }
    }

    /// The `Capabilities.ffmpeg` block for cia-core.
    pub fn capabilities(&self, installed: &InstalledFfmpeg) -> FfmpegCapabilities {
        FfmpegCapabilities {
            version: installed.version.clone(),
            path: installed.ffmpeg.to_string_lossy().into_owned(),
            user_supplied: installed.user_supplied,
            working_encoders: self.working.clone(),
            has_libx264: self.works("libx264"),
            has_libvpx_vp9: self.works("libvpx-vp9"),
            has_aac: self.works("aac"),
            has_libmp3lame: self.works("libmp3lame"),
            has_libopus: self.works("libopus"),
        }
    }

    /// The 3.5.6 chain for an H.264 target on this machine, working encoders only.
    pub fn chain(&self, faster: bool, allow_webm: bool) -> Vec<Encoder> {
        self.chain_on(platform(), faster, allow_webm)
    }

    pub fn chain_on(&self, platform: Platform, faster: bool, allow_webm: bool) -> Vec<Encoder> {
        encoder_chain(
            platform,
            self.gpu().map(GpuVendor::to_plan),
            self.works("libx264"),
            faster,
            allow_webm && self.works("libvpx-vp9"),
        )
        .into_iter()
        .filter(|name| self.works(name))
        .filter_map(Encoder::parse)
        .collect()
    }
}

/// The platform this binary runs on, for `encoder_chain`.
pub fn platform() -> Platform {
    if cfg!(target_os = "windows") {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Linux
    }
}

/// Whether an encoder can exist on this platform at all (saves a failing test).
pub fn applicable(encoder: Encoder, platform: Platform, has_dri: bool) -> bool {
    match encoder {
        Encoder::Libx264 | Encoder::LibvpxVp9 => true,
        Encoder::H264Videotoolbox => platform == Platform::MacOs,
        Encoder::H264Mf { .. } => platform == Platform::Windows,
        Encoder::H264Vaapi => platform == Platform::Linux && has_dri,
        Encoder::H264Nvenc | Encoder::H264Amf | Encoder::H264Qsv => platform != Platform::MacOs,
    }
}

/// Encoder names from `ffmpeg -hide_banner -encoders`.
pub fn parse_encoders_list(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("------") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let (Some(flags), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if flags.len() == 6 && flags.starts_with(['V', 'A', 'S']) {
            names.push(name.to_string());
        }
    }
    names
}

/// Load the cache if it describes exactly this binary.
pub fn load_cache(path: &Path, installed: &InstalledFfmpeg) -> Option<EncoderReport> {
    let text = std::fs::read_to_string(path).ok()?;
    let report: EncoderReport = serde_json::from_str(&text).ok()?;
    (report.schema == EncoderReport::SCHEMA
        && report.version == installed.version
        && report.ffmpeg_path == installed.ffmpeg.to_string_lossy())
    .then_some(report)
}

/// Cached report if valid, else run detection and write the cache.
/// `progress` receives the name of the encoder being tested.
pub fn detect(
    installed: &InstalledFfmpeg,
    cache: Option<&Path>,
    progress: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<EncoderReport, FfmpegError> {
    if let Some(report) = cache.and_then(|p| load_cache(p, installed)) {
        return Ok(report);
    }
    let report = run_detection(installed, progress, cancel)?;
    if let Some(path) = cache {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    log::warn!("could not write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("could not serialise encoder report: {e}"),
        }
    }
    Ok(report)
}

/// Always run the tests, ignoring any cache.
pub fn run_detection(
    installed: &InstalledFfmpeg,
    progress: &mut dyn FnMut(&str),
    cancel: &Cancel,
) -> Result<EncoderReport, FfmpegError> {
    let listing = run_capture(
        &installed.ffmpeg,
        encoders_list_args(),
        Duration::from_secs(30),
        Some(cancel),
    )?;
    if !listing.success() {
        return Err(FfmpegError::Exit {
            status: listing.code(),
            stderr: listing.stderr_str(),
        });
    }
    let listed = parse_encoders_list(&listing.stdout_str());
    let mut report = EncoderReport {
        schema: EncoderReport::SCHEMA,
        ffmpeg_path: installed.ffmpeg.to_string_lossy().into_owned(),
        version: installed.version.clone(),
        listed: listed.clone(),
        ..Default::default()
    };
    let platform = platform();
    let has_dri = Path::new("/dev/dri/renderD128").exists();

    for encoder in Encoder::ALL {
        let name = encoder.name();
        if !listed.iter().any(|l| l == name) || !applicable(encoder, platform, has_dri) {
            report.skipped.push(name.to_string());
            continue;
        }
        progress(name);
        match run_capture(
            &installed.ffmpeg,
            test_encode_args(encoder),
            TEST_ENCODE_DEADLINE,
            Some(cancel),
        ) {
            Ok(c) if c.success() => report.working.push(name.to_string()),
            Ok(c) => {
                report
                    .failed
                    .insert(name.to_string(), tail(&c.stderr_str()));
            }
            Err(FfmpegError::Cancelled) => return Err(FfmpegError::Cancelled),
            Err(FfmpegError::Stalled) => {
                report
                    .failed
                    .insert(name.to_string(), "test encode timed out".to_string());
            }
            Err(e) => return Err(e),
        }
    }

    for name in AUDIO_CANDIDATES {
        if !listed.iter().any(|l| l == name) {
            report.skipped.push(name.to_string());
            continue;
        }
        progress(name);
        match run_capture(
            &installed.ffmpeg,
            test_audio_encode_args(name),
            TEST_ENCODE_DEADLINE,
            Some(cancel),
        ) {
            Ok(c) if c.success() => report.working.push(name.to_string()),
            Ok(c) => {
                report
                    .failed
                    .insert(name.to_string(), tail(&c.stderr_str()));
            }
            Err(FfmpegError::Cancelled) => return Err(FfmpegError::Cancelled),
            Err(FfmpegError::Stalled) => {
                report
                    .failed
                    .insert(name.to_string(), "test encode timed out".to_string());
            }
            Err(e) => return Err(e),
        }
    }
    Ok(report)
}

fn tail(s: &str) -> String {
    let s = s.trim();
    let mut lines: Vec<&str> = s.lines().collect();
    if lines.len() > 5 {
        lines = lines[lines.len() - 5..].to_vec();
    }
    let joined = lines.join("\n");
    if joined.len() > 1000 {
        joined[joined.len() - 1000..].to_string()
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264 / AVC (codec h264)\n V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n V..... h264_mf              H264 via MediaFoundation (codec h264)\n V....D libvpx-vp9           libvpx VP9 (codec vp9)\n A..... aac                  AAC (Advanced Audio Coding)\n A..... libopus              libopus Opus (codec opus)\n S..... webvtt               WebVTT subtitle\n";

    #[test]
    fn parses_listing() {
        let names = parse_encoders_list(LISTING);
        assert_eq!(
            names,
            [
                "libx264",
                "h264_nvenc",
                "h264_mf",
                "libvpx-vp9",
                "aac",
                "libopus",
                "webvtt"
            ]
        );
        assert!(parse_encoders_list("").is_empty());
    }

    fn report(working: &[&str]) -> EncoderReport {
        EncoderReport {
            schema: EncoderReport::SCHEMA,
            working: working.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn gpu_and_chain_follow_working_encoders() {
        let r = report(&["h264_nvenc", "h264_mf", "libvpx-vp9", "aac", "libopus"]);
        assert_eq!(r.gpu(), Some(GpuVendor::Nvidia));
        let chain: Vec<&str> = r
            .chain_on(Platform::Windows, true, true)
            .iter()
            .map(|e| e.name())
            .collect();
        assert_eq!(chain, ["h264_nvenc", "h264_mf", "libvpx-vp9"]);
        let chain: Vec<&str> = r
            .chain_on(Platform::Windows, false, false)
            .iter()
            .map(|e| e.name())
            .collect();
        assert_eq!(chain, ["h264_mf"]);

        let r = report(&["libx264", "libvpx-vp9"]);
        assert_eq!(r.gpu(), None);
        let chain: Vec<&str> = r
            .chain_on(Platform::Linux, true, true)
            .iter()
            .map(|e| e.name())
            .collect();
        // vaapi is in the design chain but did not pass the test here
        assert_eq!(chain, ["libx264", "libvpx-vp9"]);

        let r = report(&["h264_amf"]);
        assert_eq!(r.gpu(), Some(GpuVendor::Amd));
        let r = report(&["h264_qsv", "h264_videotoolbox"]);
        assert_eq!(r.gpu(), Some(GpuVendor::Intel));
        let chain = r.chain_on(Platform::MacOs, true, true);
        assert_eq!(chain, [Encoder::H264Videotoolbox]);
    }

    #[test]
    fn capabilities_block() {
        let r = report(&["libx264", "aac", "libopus"]);
        let installed = InstalledFfmpeg {
            ffmpeg: "/x/ffmpeg".into(),
            ffprobe: "/x/ffprobe".into(),
            version: "7.1.2".into(),
            user_supplied: true,
            install_dir: None,
        };
        let caps = r.capabilities(&installed);
        assert!(caps.has_libx264 && caps.has_aac && caps.has_libopus);
        assert!(!caps.has_libvpx_vp9 && !caps.has_libmp3lame);
        assert!(caps.user_supplied);
        assert_eq!(caps.version, "7.1.2");
    }

    #[test]
    fn applicability() {
        assert!(applicable(Encoder::H264Vaapi, Platform::Linux, true));
        assert!(!applicable(Encoder::H264Vaapi, Platform::Linux, false));
        assert!(!applicable(Encoder::H264Vaapi, Platform::Windows, true));
        assert!(applicable(
            Encoder::H264Mf { hw_encoding: true },
            Platform::Windows,
            false
        ));
        assert!(!applicable(Encoder::H264Nvenc, Platform::MacOs, false));
        assert!(applicable(
            Encoder::H264Videotoolbox,
            Platform::MacOs,
            false
        ));
        assert!(applicable(Encoder::Libx264, Platform::MacOs, false));
    }

    #[test]
    fn cache_must_match_binary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("encoders.json");
        let installed = InstalledFfmpeg {
            ffmpeg: "/x/ffmpeg".into(),
            ffprobe: "/x/ffprobe".into(),
            version: "7.1.2".into(),
            user_supplied: false,
            install_dir: None,
        };
        let mut r = report(&["aac"]);
        r.ffmpeg_path = "/x/ffmpeg".into();
        r.version = "7.1.2".into();
        std::fs::write(&path, serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(load_cache(&path, &installed), Some(r.clone()));
        let other = InstalledFfmpeg {
            version: "7.2.0".into(),
            ..installed.clone()
        };
        assert_eq!(load_cache(&path, &other), None);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_cache(&path, &installed), None);
    }
}
