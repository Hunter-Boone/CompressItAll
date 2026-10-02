//! Compress one video (DESIGN.md 3.5.8), verify what came out (3.11), and the
//! two small decode helpers the UI needs (trim preview frame, HEIC/AVIF decode).
//!
//! [`Session`] owns the binary paths and the set of encoders that failed in
//! this session; a hardware encoder that exits non-zero or produces a file
//! that does not verify is marked broken and the next one in the chain is
//! used. Every attempt goes into the returned log.

use crate::cancel::Cancel;
use crate::command::{
    decode_check_args, decode_image_args, encode_args, extract_frame_args, partial_path,
    probe_args, remux_args, AudioOut, Edge, EncodeJob, Encoder, Pass,
};
use crate::probe::parse_probe;
use crate::process::run_capture;
use crate::runner::{remove_partials, run_ffmpeg, RunOptions};
use crate::{FfmpegError, InstalledFfmpeg};
use cia_core::presets::VideoFormat;
use cia_core::{Attempt, AttemptVerdict, AudioTrackChoice, VerificationReport};
use cia_video_plan::{
    keep_original_or_remux, plan, retry_scale, AudioCodec, Budget, Container, Decision,
    PlanOptions, PlanRefusal, Target, VideoCodec, VideoPlan, VideoProbe,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Size retries before giving up (3.5.8 `max_attempts`).
pub const DEFAULT_MAX_SIZE_ATTEMPTS: u32 = 4;

/// Share of the progress bar the first pass of a two-pass encode gets.
const FIRST_PASS_SHARE: f64 = 0.35;

/// Helper commands (decode checks, frame grabs) must finish within this.
const HELPER_DEADLINE: Duration = Duration::from_secs(120);

/// One FFmpeg plus what this session learned about its encoders.
pub struct Session {
    pub installed: InstalledFfmpeg,
    broken: Mutex<HashSet<Encoder>>,
}

impl Session {
    pub fn new(installed: InstalledFfmpeg) -> Self {
        Session {
            installed,
            broken: Mutex::new(HashSet::new()),
        }
    }

    pub fn ffmpeg(&self) -> &Path {
        &self.installed.ffmpeg
    }

    pub fn ffprobe(&self) -> &Path {
        &self.installed.ffprobe
    }

    pub fn probe(&self, input: &Path) -> Result<VideoProbe, FfmpegError> {
        crate::probe::probe(self.ffprobe(), input)
    }

    pub fn mark_broken(&self, encoder: Encoder) {
        self.broken.lock().unwrap().insert(encoder);
    }

    pub fn is_broken(&self, encoder: Encoder) -> bool {
        self.broken.lock().unwrap().contains(&encoder)
    }

    pub fn broken_encoders(&self) -> Vec<Encoder> {
        self.broken.lock().unwrap().iter().copied().collect()
    }

    pub fn verify_output(&self, path: &Path, expected: &ExpectedOutput) -> VerificationReport {
        verify_output(&self.installed, path, expected)
    }

    pub fn extract_frame(&self, input: &Path, ms: u64) -> Result<Vec<u8>, FfmpegError> {
        extract_frame(&self.installed, input, ms)
    }

    pub fn decode_image(&self, input: &Path) -> Result<image::RgbaImage, FfmpegError> {
        decode_image(&self.installed, input)
    }

    /// The 3.5.8 loop. Never panics on FFmpeg failures; the outcome says what happened.
    pub fn compress_video(
        &self,
        req: &CompressRequest<'_>,
        progress: &mut dyn FnMut(f64, Option<u64>),
        cancel: &Cancel,
    ) -> VideoResult {
        let mut result = VideoResult {
            outcome: VideoOutcome::Cancelled,
            attempts: Vec::new(),
            plan: None,
            encoder: None,
        };
        let hard_bytes = req.budget.hard_bytes;

        if !req.allowed_formats.is_empty() {
            match keep_original_or_remux(
                req.probe,
                hard_bytes,
                req.source_bytes,
                req.allowed_formats,
            ) {
                Decision::KeepOriginal => {
                    result.outcome = VideoOutcome::KeptOriginal;
                    return result;
                }
                Decision::Remux { container } => {
                    match self.try_remux(req, container, hard_bytes, &mut result, progress, cancel)
                    {
                        Ok(Some(outcome)) => {
                            result.outcome = outcome;
                            return result;
                        }
                        Ok(None) => {}
                        Err(outcome) => {
                            result.outcome = outcome;
                            return result;
                        }
                    }
                }
                Decision::Encode => {}
            }
        }

        let mut chain = req
            .encoders
            .iter()
            .copied()
            .filter(|e| !self.is_broken(*e))
            .collect::<Vec<_>>()
            .into_iter();
        let mut current = chain.next();
        let mut current_plan: Option<VideoPlan> = None;
        let mut size_attempts = 0u32;
        let mut closest: Option<u64> = None;
        let max_size_attempts = req.max_size_attempts.max(1);

        loop {
            if cancel.is_cancelled() {
                result.outcome = VideoOutcome::Cancelled;
                return result;
            }
            let Some(encoder) = current else {
                result.outcome = match closest {
                    Some(c) => failed("over_after_retries", Some(c), hard_bytes),
                    None if req.encoders.is_empty() => failed("no_encoder", None, hard_bytes),
                    None => failed("encoder_crash", None, hard_bytes),
                };
                return result;
            };
            result.encoder = Some(encoder);
            let target = target_for(encoder, req.target);
            if current_plan.is_none() {
                match plan(req.probe, req.budget, &target, encoder.kind(), req.options) {
                    Ok(p) => current_plan = Some(p),
                    Err(refusal) => {
                        result.outcome = VideoOutcome::Refused(refusal);
                        return result;
                    }
                }
            }
            let the_plan = current_plan.clone().expect("plan set above");
            result.plan = Some(the_plan.clone());

            let final_path =
                req.sink_dir
                    .join(format!("{}.{}", req.output_stem, target.container.token()));
            let partial = partial_path(&final_path);
            let n = result.attempts.len() as u32 + 1;
            let started = Instant::now();
            let run = self.run_encode(req, &the_plan, encoder, &partial, progress, cancel);
            let mut attempt = Attempt {
                n,
                item_id: req.item_id.clone(),
                encoder: encoder.attempt_label(),
                params: plan_params(&the_plan, encoder),
                output_bytes: None,
                score: None,
                elapsed_ms: started.elapsed().as_millis() as u64,
                verdict: AttemptVerdict::Cancelled,
            };

            match run {
                Err(FfmpegError::Cancelled) => {
                    remove_partials(&[partial]);
                    result.attempts.push(attempt);
                    result.outcome = VideoOutcome::Cancelled;
                    return result;
                }
                Err(FfmpegError::DiskFull) => {
                    remove_partials(&[partial]);
                    attempt.verdict = AttemptVerdict::Error {
                        message: "disk full".to_string(),
                    };
                    result.attempts.push(attempt);
                    result.outcome = failed("disk_full", None, hard_bytes);
                    return result;
                }
                Err(e) => {
                    remove_partials(&[partial]);
                    attempt.verdict = AttemptVerdict::Error {
                        message: e.to_string(),
                    };
                    result.attempts.push(attempt);
                    log::warn!("{} failed: {e}", encoder.name());
                    if let Encoder::H264Mf { hw_encoding: true } = encoder {
                        // 3.5.7: fall back to -hw_encoding 0 before giving up on MF.
                        current = Some(Encoder::H264Mf { hw_encoding: false });
                        continue;
                    }
                    if encoder.is_hardware() {
                        self.mark_broken(encoder);
                        current = chain.next();
                        current_plan = None;
                        continue;
                    }
                    result.outcome = failed(e.failure_code(), None, hard_bytes);
                    return result;
                }
                Ok(()) => {}
            }

            let actual = match std::fs::metadata(&partial) {
                Ok(m) => m.len(),
                Err(e) => {
                    attempt.verdict = AttemptVerdict::Error {
                        message: format!("output missing: {e}"),
                    };
                    result.attempts.push(attempt);
                    result.outcome = failed("encoder_crash", None, hard_bytes);
                    return result;
                }
            };
            attempt.output_bytes = Some(actual);
            closest = Some(closest.map_or(actual, |c| c.min(actual)));

            let expected = ExpectedOutput::from_plan(&the_plan, hard_bytes);
            let report = self.verify_output(&partial, &expected);

            if actual <= hard_bytes && report.passed() {
                attempt.verdict = AttemptVerdict::Fits;
                attempt.elapsed_ms = started.elapsed().as_millis() as u64;
                result.attempts.push(attempt);
                if final_path.exists() {
                    let _ = std::fs::remove_file(&final_path);
                }
                if let Err(e) = std::fs::rename(&partial, &final_path) {
                    remove_partials(&[partial]);
                    result.outcome = failed(FfmpegError::Io(e).failure_code(), None, hard_bytes);
                    return result;
                }
                result.outcome = VideoOutcome::Fitted {
                    path: final_path,
                    bytes: actual,
                    verification: report,
                    remuxed: false,
                };
                return result;
            }

            remove_partials(&[partial]);
            if !report.passed() && report.size_ok {
                attempt.verdict = AttemptVerdict::Error {
                    message: format!("verification failed: {}", report.failures.join("; ")),
                };
                result.attempts.push(attempt);
                if encoder.is_hardware() {
                    self.mark_broken(encoder);
                    current = chain.next();
                    current_plan = None;
                    continue;
                }
                result.outcome = failed("verify_failed", None, hard_bytes);
                return result;
            }

            // Over budget (verification may also have flagged the size).
            attempt.verdict = AttemptVerdict::Over {
                by_bytes: actual.saturating_sub(hard_bytes),
            };
            result.attempts.push(attempt);
            size_attempts += 1;
            if size_attempts >= max_size_attempts {
                result.outcome = failed("over_after_retries", closest, hard_bytes);
                return result;
            }
            match retry_scale(&the_plan, actual, req.budget, encoder.kind()) {
                Some(next) => current_plan = Some(next),
                None => {
                    if let Some(last) = result.attempts.last_mut() {
                        last.verdict = AttemptVerdict::BelowFloor;
                    }
                    result.outcome = failed("over_after_retries", closest, hard_bytes);
                    return result;
                }
            }
        }
    }

    /// `Ok(Some)` when the remux fit and verified, `Ok(None)` to fall through
    /// to an encode, `Err` for cancel.
    fn try_remux(
        &self,
        req: &CompressRequest<'_>,
        container: Container,
        hard_bytes: u64,
        result: &mut VideoResult,
        progress: &mut dyn FnMut(f64, Option<u64>),
        cancel: &Cancel,
    ) -> Result<Option<VideoOutcome>, VideoOutcome> {
        let final_path = req
            .sink_dir
            .join(format!("{}.{}", req.output_stem, container.token()));
        let partial = partial_path(&final_path);
        let audio_track = match req.options.audio {
            AudioTrackChoice::Remove => None,
            AudioTrackChoice::Track { index }
                if req.probe.audio.iter().any(|t| t.index == index) =>
            {
                Some(index)
            }
            _ => req.probe.audio.first().map(|t| t.index),
        };
        let args = remux_args(req.input, &partial, container, audio_track);
        let opts = RunOptions {
            duration_ms: Some(req.probe.duration_ms),
            partial_files: vec![partial.clone()],
            ..RunOptions::default()
        };
        let started = Instant::now();
        let run = run_ffmpeg(self.ffmpeg(), &args, &opts, progress, cancel);
        let mut attempt = Attempt {
            n: result.attempts.len() as u32 + 1,
            item_id: req.item_id.clone(),
            encoder: "remux".to_string(),
            params: serde_json::json!({ "container": container.token() }),
            output_bytes: None,
            score: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
            verdict: AttemptVerdict::Cancelled,
        };
        match run {
            Err(FfmpegError::Cancelled) => {
                result.attempts.push(attempt);
                Err(VideoOutcome::Cancelled)
            }
            Err(e) => {
                attempt.verdict = AttemptVerdict::Error {
                    message: e.to_string(),
                };
                result.attempts.push(attempt);
                Ok(None)
            }
            Ok(_) => {
                let actual = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
                attempt.output_bytes = Some(actual);
                let video_codec =
                    VideoCodec::parse(&req.probe.video_codec).unwrap_or(VideoCodec::H264);
                let expected = ExpectedOutput {
                    container,
                    video_codec,
                    duration_ms: req.probe.duration_ms,
                    fps: req.probe.avg_fps,
                    width: req.probe.display_w,
                    height: req.probe.display_h,
                    has_audio: audio_track.is_some(),
                    hard_bytes,
                };
                let report = self.verify_output(&partial, &expected);
                if actual > 0 && actual <= hard_bytes && report.passed() {
                    attempt.verdict = AttemptVerdict::Fits;
                    result.attempts.push(attempt);
                    if final_path.exists() {
                        let _ = std::fs::remove_file(&final_path);
                    }
                    if std::fs::rename(&partial, &final_path).is_err() {
                        remove_partials(&[partial]);
                        return Ok(None);
                    }
                    return Ok(Some(VideoOutcome::Fitted {
                        path: final_path,
                        bytes: actual,
                        verification: report,
                        remuxed: true,
                    }));
                }
                remove_partials(&[partial]);
                attempt.verdict = if actual > hard_bytes {
                    AttemptVerdict::Over {
                        by_bytes: actual - hard_bytes,
                    }
                } else {
                    AttemptVerdict::Error {
                        message: format!(
                            "remux verification failed: {}",
                            report.failures.join("; ")
                        ),
                    }
                };
                result.attempts.push(attempt);
                Ok(None)
            }
        }
    }

    /// One or two ffmpeg runs for `plan` with `encoder`, writing `partial`.
    fn run_encode(
        &self,
        req: &CompressRequest<'_>,
        plan: &VideoPlan,
        encoder: Encoder,
        partial: &Path,
        progress: &mut dyn FnMut(f64, Option<u64>),
        cancel: &Cancel,
    ) -> Result<(), FfmpegError> {
        let audio = if plan.has_audio() {
            let tracks = audio_tracks(req.probe, &req.options.audio);
            (!tracks.is_empty()).then(|| AudioOut {
                codec: plan.audio_codec,
                bps: plan.audio_bps,
                channels: plan.audio_channels.max(1),
                tracks,
            })
        } else {
            None
        };
        let base_opts = RunOptions {
            duration_ms: Some(plan.duration_ms),
            partial_files: vec![partial.to_path_buf()],
            ..RunOptions::default()
        };
        if encoder.two_pass() {
            let dir = tempfile::Builder::new()
                .prefix("smidge-pass-")
                .tempdir()
                .map_err(FfmpegError::Io)?;
            let passlog = dir.path().join("pass");
            let first = encode_args(&make_job(
                req,
                plan,
                encoder,
                partial,
                audio.clone(),
                Pass::First { passlog: &passlog },
            ));
            let opts = RunOptions {
                fraction_base: 0.0,
                fraction_span: FIRST_PASS_SHARE,
                ..base_opts.clone()
            };
            run_ffmpeg(self.ffmpeg(), &first, &opts, progress, cancel)?;
            let second = encode_args(&make_job(
                req,
                plan,
                encoder,
                partial,
                audio.clone(),
                Pass::Second { passlog: &passlog },
            ));
            let opts = RunOptions {
                fraction_base: FIRST_PASS_SHARE,
                fraction_span: 1.0 - FIRST_PASS_SHARE,
                ..base_opts
            };
            run_ffmpeg(self.ffmpeg(), &second, &opts, progress, cancel)?;
        } else {
            let args = encode_args(&make_job(req, plan, encoder, partial, audio, Pass::Single));
            run_ffmpeg(self.ffmpeg(), &args, &base_opts, progress, cancel)?;
        }
        Ok(())
    }
}

fn make_job<'a>(
    req: &CompressRequest<'a>,
    plan: &VideoPlan,
    encoder: Encoder,
    partial: &'a Path,
    audio: Option<AudioOut>,
    pass: Pass<'a>,
) -> EncodeJob<'a> {
    EncodeJob {
        input: req.input,
        output: partial,
        trim: req.options.trim,
        width: plan.width,
        height: plan.height,
        fps: plan.fps,
        video_bps: plan.video_bps,
        hdr: req.probe.is_hdr,
        encoder,
        container: plan.container,
        audio,
        pass,
    }
}

/// Everything `compress_video` needs for one item.
#[derive(Debug, Clone)]
pub struct CompressRequest<'a> {
    pub item_id: String,
    pub input: &'a Path,
    pub source_bytes: u64,
    pub probe: &'a VideoProbe,
    pub budget: &'a Budget,
    /// The preset's preferred target; container, codecs follow the encoder actually used.
    pub target: &'a Target,
    pub options: &'a PlanOptions,
    /// Encoders in chain order (from `EncoderReport::chain`).
    pub encoders: &'a [Encoder],
    /// What the preset accepts, for keep-original / remux. Empty: always encode.
    pub allowed_formats: &'a [VideoFormat],
    pub sink_dir: &'a Path,
    /// Output file name without extension.
    pub output_stem: &'a str,
    pub max_size_attempts: u32,
}

/// What `compress_video` ended with.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoOutcome {
    Fitted {
        path: PathBuf,
        bytes: u64,
        verification: VerificationReport,
        /// `-c copy` into the right container, no re-encode.
        remuxed: bool,
    },
    /// The source already fits and its codecs are allowed; nothing written.
    KeptOriginal,
    Refused(PlanRefusal),
    Failed {
        code: String,
        message: String,
        closest_bytes: Option<u64>,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct VideoResult {
    pub outcome: VideoOutcome,
    pub attempts: Vec<Attempt>,
    /// The last plan that was encoded.
    pub plan: Option<VideoPlan>,
    /// The last encoder that was tried.
    pub encoder: Option<Encoder>,
}

/// What the output must look like for the 3.11 video row.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedOutput {
    pub container: Container,
    pub video_codec: VideoCodec,
    pub duration_ms: u64,
    pub fps: f32,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub hard_bytes: u64,
}

impl ExpectedOutput {
    pub fn from_plan(plan: &VideoPlan, hard_bytes: u64) -> Self {
        ExpectedOutput {
            container: plan.container,
            video_codec: plan.video_codec,
            duration_ms: plan.duration_ms,
            fps: plan.fps,
            width: plan.width,
            height: plan.height,
            has_audio: plan.has_audio(),
            hard_bytes,
        }
    }

    /// `max(0.1 s, one frame)` in milliseconds.
    pub fn duration_tolerance_ms(&self) -> u64 {
        let frame = if self.fps > 0.0 {
            (1000.0 / self.fps).ceil() as u64
        } else {
            0
        };
        frame.max(100)
    }
}

fn failed(code: &str, closest: Option<u64>, hard_bytes: u64) -> VideoOutcome {
    VideoOutcome::Failed {
        code: code.to_string(),
        message: cia_core::copy::failure_message(code, closest, Some(hard_bytes)),
        closest_bytes: closest,
    }
}

/// Container and codecs follow the encoder; caps follow the preset.
pub fn target_for(encoder: Encoder, preset_target: &Target) -> Target {
    let container = encoder.container();
    let audio = match container {
        Container::Mp4 => AudioCodec::Aac,
        Container::Webm => AudioCodec::Opus,
    };
    Target::new(container, encoder.video_codec(), audio).with_caps(preset_target.caps.clone())
}

/// Source audio stream indices the output mixes or copies.
pub fn audio_tracks(probe: &VideoProbe, choice: &AudioTrackChoice) -> Vec<u32> {
    match choice {
        AudioTrackChoice::Remove => Vec::new(),
        AudioTrackChoice::Track { index } if probe.audio.iter().any(|t| t.index == *index) => {
            vec![*index]
        }
        _ => probe.audio.iter().map(|t| t.index).collect(),
    }
}

fn plan_params(plan: &VideoPlan, encoder: Encoder) -> serde_json::Value {
    serde_json::json!({
        "encoder": encoder.name(),
        "two_pass": encoder.two_pass(),
        "width": plan.width,
        "height": plan.height,
        "fps": plan.fps,
        "video_bps": plan.video_bps,
        "audio_bps": plan.audio_bps,
        "audio_channels": plan.audio_channels,
        "container": plan.container.token(),
        "rung_index": plan.rung_index,
        "predicted_bytes": plan.predicted_bytes,
        "quality": plan.quality.word(),
    })
}

/// The video row of the 3.11 table, against the bytes on disk.
pub fn verify_output(
    installed: &InstalledFfmpeg,
    path: &Path,
    expected: &ExpectedOutput,
) -> VerificationReport {
    let mut r = VerificationReport::default();
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => {
            r.failures.push(format!("output missing: {e}"));
            return r;
        }
    };
    r.size_ok = size <= expected.hard_bytes;
    if r.size_ok {
        r.checks
            .push(format!("size {size} <= {}", expected.hard_bytes));
    } else {
        r.failures
            .push(format!("size {size} > {}", expected.hard_bytes));
    }

    let probe_run = run_capture(&installed.ffprobe, probe_args(path), HELPER_DEADLINE, None);
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let out = match probe_run {
        Ok(c) if c.success() => match parse_probe(&c.stdout_str(), None, file_name.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                r.failures.push(format!("ffprobe output unusable: {e}"));
                return r;
            }
        },
        Ok(c) => {
            r.failures.push(format!(
                "ffprobe cannot read the output: {}",
                c.stderr_str().trim()
            ));
            return r;
        }
        Err(e) => {
            r.failures.push(format!("ffprobe failed: {e}"));
            return r;
        }
    };
    r.checks.push("ffprobe reads it".to_string());

    let want_container = expected.container.token();
    if out.container == want_container {
        r.checks.push(format!("container {want_container}"));
    } else {
        r.failures
            .push(format!("container {} not {want_container}", out.container));
    }
    let want_codec = expected.video_codec.token();
    if out.video_codec == want_codec {
        r.checks.push(format!("codec {want_codec}"));
    } else {
        r.failures
            .push(format!("codec {} not {want_codec}", out.video_codec));
    }

    let tol = expected.duration_tolerance_ms();
    let diff = out.duration_ms.abs_diff(expected.duration_ms);
    if diff <= tol {
        r.checks
            .push(format!("duration {} ms within {tol} ms", out.duration_ms));
    } else {
        r.failures.push(format!(
            "duration {} ms is {diff} ms from {} (tolerance {tol})",
            out.duration_ms, expected.duration_ms
        ));
    }

    if expected.width > 0 && expected.height > 0 && out.display_w > 0 && out.display_h > 0 {
        let want = f64::from(expected.width) / f64::from(expected.height);
        let got = f64::from(out.display_w) / f64::from(out.display_h);
        if ((got - want) / want).abs() <= 0.01 {
            r.checks
                .push(format!("aspect {}x{}", out.display_w, out.display_h));
        } else {
            r.failures.push(format!(
                "aspect {}x{} differs from {}x{} by more than 1 percent",
                out.display_w, out.display_h, expected.width, expected.height
            ));
        }
        let want_portrait = expected.height > expected.width;
        let got_portrait = out.display_h > out.display_w;
        if want_portrait && !got_portrait {
            r.failures
                .push("portrait source came out landscape".to_string());
        } else if want_portrait {
            r.checks.push("portrait stays portrait".to_string());
        }
        if out.rotation_degrees != 0 {
            r.failures
                .push(format!("output carries rotation {}", out.rotation_degrees));
        }
    } else {
        r.failures.push("no picture size".to_string());
    }

    let has_audio = !out.audio.is_empty();
    if has_audio == expected.has_audio {
        r.checks.push(if has_audio {
            format!("{} audio stream(s)", out.audio.len())
        } else {
            "no audio, as planned".to_string()
        });
        if expected.has_audio && out.audio.len() != 1 {
            r.failures
                .push(format!("{} audio streams, expected 1", out.audio.len()));
        }
    } else {
        r.failures.push(if has_audio {
            "audio present but none planned".to_string()
        } else {
            "audio missing".to_string()
        });
    }

    let mut decodes = true;
    for edge in [Edge::First, Edge::Last] {
        match run_capture(
            &installed.ffmpeg,
            decode_check_args(path, edge),
            HELPER_DEADLINE,
            None,
        ) {
            Ok(c) if c.success() && c.stderr_str().trim().is_empty() => {
                r.checks.push(format!("{edge:?} second decodes"));
            }
            Ok(c) => {
                decodes = false;
                r.failures.push(format!(
                    "{edge:?} second does not decode cleanly: {}",
                    c.stderr_str().trim()
                ));
            }
            Err(e) => {
                decodes = false;
                r.failures
                    .push(format!("{edge:?} second decode check failed: {e}"));
            }
        }
    }
    r.decodes = decodes;
    r
}

/// One JPEG frame at `ms` for the trim preview.
pub fn extract_frame(
    installed: &InstalledFfmpeg,
    input: &Path,
    ms: u64,
) -> Result<Vec<u8>, FfmpegError> {
    let c = run_capture(
        &installed.ffmpeg,
        extract_frame_args(input, ms),
        HELPER_DEADLINE,
        None,
    )?;
    if !c.success() {
        return Err(FfmpegError::Exit {
            status: c.code(),
            stderr: c.stderr_str().trim().to_string(),
        });
    }
    if c.stdout.len() < 4 || c.stdout[..2] != [0xFF, 0xD8] {
        return Err(FfmpegError::BadOutput(
            "no JPEG frame came back (position past the end?)".to_string(),
        ));
    }
    Ok(c.stdout)
}

/// Decode a still image FFmpeg can read (HEIC, AVIF, anything else) to RGBA.
pub fn decode_image(
    installed: &InstalledFfmpeg,
    input: &Path,
) -> Result<image::RgbaImage, FfmpegError> {
    let c = run_capture(
        &installed.ffmpeg,
        decode_image_args(input),
        HELPER_DEADLINE,
        None,
    )?;
    if !c.success() {
        return Err(FfmpegError::Exit {
            status: c.code(),
            stderr: c.stderr_str().trim().to_string(),
        });
    }
    let img = image::load_from_memory_with_format(&c.stdout, image::ImageFormat::Png)
        .map_err(|e| FfmpegError::BadOutput(format!("PNG from ffmpeg: {e}")))?;
    Ok(img.to_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cia_video_plan::AudioTrack;

    fn probe_with_tracks(n: u32) -> VideoProbe {
        VideoProbe {
            audio: (0..n)
                .map(|i| AudioTrack {
                    index: i,
                    codec: "aac".into(),
                    channels: 2,
                    sample_rate: 48_000,
                    bitrate_bps: None,
                    title: None,
                })
                .collect(),
            ..VideoProbe::default()
        }
    }

    #[test]
    fn audio_track_selection() {
        let p = probe_with_tracks(2);
        assert_eq!(audio_tracks(&p, &AudioTrackChoice::MixAll), vec![0, 1]);
        assert_eq!(
            audio_tracks(&p, &AudioTrackChoice::Track { index: 1 }),
            vec![1]
        );
        // missing track falls back to the mix
        assert_eq!(
            audio_tracks(&p, &AudioTrackChoice::Track { index: 7 }),
            vec![0, 1]
        );
        assert!(audio_tracks(&p, &AudioTrackChoice::Remove).is_empty());
        assert!(audio_tracks(&probe_with_tracks(0), &AudioTrackChoice::MixAll).is_empty());
    }

    #[test]
    fn target_follows_encoder() {
        let preset = Target::new(Container::Mp4, VideoCodec::H264, AudioCodec::Aac).with_caps(
            cia_core::presets::VideoCaps {
                max_short_edge: Some(720),
                max_fps: Some(30),
            },
        );
        let t = target_for(Encoder::LibvpxVp9, &preset);
        assert_eq!(t.container, Container::Webm);
        assert_eq!(t.video_codec, VideoCodec::Vp9);
        assert_eq!(t.audio_codec, AudioCodec::Opus);
        assert_eq!(t.caps.max_short_edge, Some(720));
        let t = target_for(Encoder::H264Nvenc, &preset);
        assert_eq!(t.container, Container::Mp4);
        assert_eq!(t.audio_codec, AudioCodec::Aac);
    }

    #[test]
    fn duration_tolerance() {
        let mut e = ExpectedOutput {
            container: Container::Mp4,
            video_codec: VideoCodec::H264,
            duration_ms: 10_000,
            fps: 30.0,
            width: 1280,
            height: 720,
            has_audio: true,
            hard_bytes: 1,
        };
        assert_eq!(e.duration_tolerance_ms(), 100);
        e.fps = 5.0;
        assert_eq!(e.duration_tolerance_ms(), 200);
        e.fps = 0.0;
        assert_eq!(e.duration_tolerance_ms(), 100);
    }

    #[test]
    fn session_tracks_broken_encoders() {
        let s = Session::new(InstalledFfmpeg {
            ffmpeg: "/x/ffmpeg".into(),
            ffprobe: "/x/ffprobe".into(),
            version: "0".into(),
            user_supplied: true,
            install_dir: None,
        });
        assert!(!s.is_broken(Encoder::H264Nvenc));
        s.mark_broken(Encoder::H264Nvenc);
        assert!(s.is_broken(Encoder::H264Nvenc));
        assert_eq!(s.broken_encoders(), vec![Encoder::H264Nvenc]);
    }

    #[test]
    fn failed_outcome_uses_core_copy() {
        let VideoOutcome::Failed {
            message,
            closest_bytes,
            ..
        } = failed("over_after_retries", Some(21_300_000), 20_905_984)
        else {
            panic!()
        };
        assert!(message.contains("couldn't get this under"), "{message}");
        assert_eq!(closest_bytes, Some(21_300_000));
    }
}
