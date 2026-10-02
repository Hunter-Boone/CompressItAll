//! The desktop `VideoBackend`: cia-ffmpeg driven by the engine (feature `ffmpeg`).

use crate::output::OutputDest;
use crate::{CancelToken, EngineError, VideoBackend, VideoTranscodeResult};
use cia_core::*;
use cia_ffmpeg::transcode::{CompressRequest, Session, VideoOutcome};
use cia_ffmpeg::{EncoderReport, InstalledFfmpeg};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct FfmpegBackend {
    pub session: Arc<Session>,
    pub report: EncoderReport,
    pub app_data: PathBuf,
}

impl FfmpegBackend {
    /// Locate an installed FFmpeg (or `CIA_FFMPEG`), detect encoders (cached), and build the backend.
    pub fn locate(app_data: &Path) -> Option<Self> {
        let installed = cia_ffmpeg::locate(app_data)?;
        Self::from_installed(installed, app_data).ok()
    }
    pub fn from_installed(
        installed: InstalledFfmpeg,
        app_data: &Path,
    ) -> Result<Self, EngineError> {
        let cache = installed.encoders_cache_path(app_data);
        let mut noop = |_: &str| {};
        let report = cia_ffmpeg::encoders::detect(
            &installed,
            Some(&cache),
            &mut noop,
            &cia_ffmpeg::Cancel::new(),
        )
        .map_err(|e| EngineError::Other(e.to_string()))?;
        Ok(Self {
            session: Arc::new(Session::new(installed)),
            report,
            app_data: app_data.to_path_buf(),
        })
    }
    pub fn allowed_webm(target: &cia_video_plan::Target) -> bool {
        matches!(target.container, cia_video_plan::Container::Webm)
    }
    fn path_of(source: &SourceRef) -> Result<PathBuf, EngineError> {
        match source {
            SourceRef::Path { path } => Ok(PathBuf::from(path)),
            SourceRef::Handle { .. } => Err(EngineError::Unsupported("video from a handle".into())),
        }
    }
}

impl VideoBackend for FfmpegBackend {
    fn probe(&self, source: &SourceRef) -> Result<cia_video_plan::VideoProbe, EngineError> {
        let p = Self::path_of(source)?;
        self.session
            .probe(&p)
            .map_err(|e| EngineError::Damaged(e.to_string()))
    }

    fn transcode(
        &self,
        source: &SourceRef,
        probe: &cia_video_plan::VideoProbe,
        budget: Option<cia_video_plan::Budget>,
        target: cia_video_plan::Target,
        options: &cia_video_plan::PlanOptions,
        faster: bool,
        dest: &OutputDest,
        progress: &dyn Fn(f32, Option<u64>),
        cancel: &CancelToken,
    ) -> Result<VideoTranscodeResult, EngineError> {
        let input = Self::path_of(source)?;
        let source_bytes = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
        let budget = budget.unwrap_or(cia_video_plan::Budget {
            raw_budget_bytes: source_bytes * 6 / 10,
            hard_bytes: source_bytes,
            safety_bytes: 0,
        });
        let encoders = self.report.chain(faster, Self::allowed_webm(&target));
        let sink_dir = if dest.dir.is_empty() {
            std::env::temp_dir()
        } else {
            PathBuf::from(&dest.dir)
        };
        let stem = format!(".smidge-video-{}", cia_core::new_id());
        let allowed: Vec<cia_core::presets::VideoFormat> = vec![];
        let req = CompressRequest {
            item_id: String::new(),
            input: &input,
            source_bytes,
            probe,
            budget: &budget,
            target: &target,
            options,
            encoders: &encoders,
            allowed_formats: &allowed,
            sink_dir: &sink_dir,
            output_stem: &stem,
            max_size_attempts: 4,
        };
        let ff_cancel = cia_ffmpeg::Cancel::new();
        let token = cancel.clone();
        let c2 = ff_cancel.clone();
        // Bridge the engine's token to FFmpeg's cancel without a thread: poll inside progress.
        let mut prog = |f: f64, eta: Option<u64>| {
            if token.is_cancelled() {
                c2.cancel();
            }
            progress(f as f32, eta);
        };
        let r = self.session.compress_video(&req, &mut prog, &ff_cancel);
        match r.outcome {
            VideoOutcome::Fitted {
                path,
                bytes,
                verification,
                remuxed,
            } => Ok(VideoTranscodeResult {
                location: OutputLocation::Path {
                    path: path.to_string_lossy().to_string(),
                },
                file_name: path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                bytes,
                plan: Some(r.plan.ok_or_else(|| EngineError::Other("no plan".into()))?),
                attempts: r.attempts,
                verification,
                kept_original: false,
            })
            .map(|mut v| {
                let _ = remuxed;
                v.kept_original = false;
                v
            }),
            VideoOutcome::KeptOriginal => Ok(VideoTranscodeResult {
                location: OutputLocation::Path {
                    path: input.to_string_lossy().to_string(),
                },
                file_name: String::new(),
                bytes: source_bytes,
                plan: r.plan,
                attempts: r.attempts,
                verification: VerificationReport {
                    size_ok: true,
                    decodes: true,
                    checks: vec!["original kept".into()],
                    failures: vec![],
                },
                kept_original: true,
            }),
            VideoOutcome::Refused(refusal) => Err(EngineError::Other(format!(
                "refused:{}",
                match refusal.code {
                    RefusalCode::TooLongForLimit { max_duration_ms } => max_duration_ms,
                    _ => 0,
                }
            ))),
            VideoOutcome::Failed {
                code,
                message,
                closest_bytes,
            } => {
                if code == "over_after_retries" {
                    Err(EngineError::Other(format!(
                        "over:{}",
                        closest_bytes.unwrap_or(0)
                    )))
                } else if code == "damaged_input" || code == "damaged_video" {
                    Err(EngineError::Damaged(message))
                } else {
                    Err(EngineError::Other(format!("{code}: {message}")))
                }
            }
            VideoOutcome::Cancelled => Err(EngineError::Cancelled),
        }
    }

    fn encoder_kind(&self, faster: bool) -> cia_video_plan::EncoderKind {
        let chain = self.report.chain(faster, true);
        match chain.first() {
            Some(e) if e.two_pass() => cia_video_plan::EncoderKind::TwoPassSoftware,
            _ => cia_video_plan::EncoderKind::SinglePassHardware,
        }
    }

    fn capabilities(&self) -> Option<FfmpegCapabilities> {
        Some(self.report.capabilities(&self.session.installed))
    }
}
