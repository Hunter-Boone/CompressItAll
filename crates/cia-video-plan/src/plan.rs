//! The planner proper: target, options, plan, refusal (DESIGN.md 3.5.2 to 3.5.5).

use crate::audio::{min_total_for_video, pick_audio, video_audio_ladder, AudioChoice};
use crate::budget::{container_overhead, total_bps, Budget, Margin};
use crate::ladder::{build_rungs, floor_bpp, quality_label, Rung};
use crate::probe::VideoProbe;
use cia_core::presets::{Preset, VideoCaps, VideoFormat};
use cia_core::{AudioTrackChoice, FrameRatePref, QualityLabel, RefusalCode, Suggestion};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Container {
    Mp4,
    Webm,
}

impl Container {
    pub fn token(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Webm => "webm",
        }
    }
    pub fn parse(token: &str) -> Option<Container> {
        match token.to_ascii_lowercase().as_str() {
            "mp4" => Some(Container::Mp4),
            "webm" => Some(Container::Webm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VideoCodec {
    H264,
    Vp9,
    Av1,
}

impl VideoCodec {
    pub fn token(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264",
            VideoCodec::Vp9 => "vp9",
            VideoCodec::Av1 => "av1",
        }
    }
    pub fn parse(token: &str) -> Option<VideoCodec> {
        match token.to_ascii_lowercase().as_str() {
            "h264" | "avc" | "avc1" => Some(VideoCodec::H264),
            "vp9" | "vp09" => Some(VideoCodec::Vp9),
            "av1" | "av01" => Some(VideoCodec::Av1),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AudioCodec {
    Aac,
    Opus,
}

impl AudioCodec {
    pub fn token(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
        }
    }
    pub fn parse(token: &str) -> Option<AudioCodec> {
        match token.to_ascii_lowercase().as_str() {
            "aac" | "mp4a" => Some(AudioCodec::Aac),
            "opus" => Some(AudioCodec::Opus),
            _ => None,
        }
    }
}

/// How the video will be encoded; decides the margin and the retry factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EncoderKind {
    /// libx264 or libvpx-vp9 with a first pass: margin 0.96, retry factor 0.97.
    TwoPassSoftware,
    /// NVENC, AMF, QSV, VA-API, VideoToolbox, Media Foundation: margin 0.92, retry 0.93.
    SinglePassHardware,
    /// Browser `VideoEncoder`: treated like single-pass hardware.
    WebCodecs,
}

impl EncoderKind {
    pub fn two_pass(self) -> bool {
        matches!(self, EncoderKind::TwoPassSoftware)
    }
}

/// What the preset accepts: container, codecs and size/frame-rate caps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Target {
    pub container: Container,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub caps: VideoCaps,
}

impl Target {
    pub fn new(container: Container, video_codec: VideoCodec, audio_codec: AudioCodec) -> Self {
        Target {
            container,
            video_codec,
            audio_codec,
            caps: VideoCaps::default(),
        }
    }

    pub fn with_caps(mut self, caps: VideoCaps) -> Self {
        self.caps = caps;
        self
    }

    /// None when the format names a container or codec this crate does not plan for.
    pub fn from_format(format: &VideoFormat, caps: &VideoCaps) -> Option<Target> {
        Some(Target {
            container: Container::parse(&format.container)?,
            video_codec: VideoCodec::parse(&format.video)?,
            audio_codec: AudioCodec::parse(&format.audio)?,
            caps: caps.clone(),
        })
    }

    /// The preset's first (preferred) video format, with its caps.
    pub fn from_preset(preset: &Preset) -> Option<Target> {
        preset
            .formats
            .video
            .iter()
            .find_map(|f| Target::from_format(f, &preset.video_caps))
    }
}

/// User choices from the Advanced drawer that change the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct PlanOptions {
    pub audio: AudioTrackChoice,
    pub frame_rate_pref: FrameRatePref,
    /// `(start_ms, end_ms)` in source time; the plan covers only this range.
    #[ts(type = "[number, number] | null")]
    pub trim: Option<(u64, u64)>,
}

/// The plan for one video: what to encode and what to expect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoPlan {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub video_bps: u64,
    /// 0 when the output has no audio.
    pub audio_bps: u64,
    /// 0 when the output has no audio.
    pub audio_channels: u32,
    pub container: Container,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub two_pass: bool,
    pub quality: QualityLabel,
    pub predicted_bytes: u64,
    /// Index of the chosen rung in `ladder`.
    pub rung_index: usize,
    /// The whole ladder, so a retry can step down without re-probing.
    pub ladder: Vec<Rung>,
    /// Output duration (after trim) the numbers were computed for.
    pub duration_ms: u64,
    pub overhead_bytes: u64,
    /// Bits per pixel per frame of the video stream at the chosen rung.
    pub bpp: f64,
    pub floor_bpp: f64,
}

impl VideoPlan {
    pub fn rung(&self) -> Rung {
        Rung {
            width: self.width,
            height: self.height,
            fps: self.fps,
        }
    }
    pub fn has_audio(&self) -> bool {
        self.audio_bps > 0
    }
    pub fn total_bps(&self) -> u64 {
        self.video_bps + self.audio_bps
    }
}

/// Why no plan exists. `code` is `TooLongForLimit` when a shorter clip would
/// fit (with a `Trim` suggestion), `BelowQualityFloor` when not even one
/// second fits, `UnsupportedInput` for a probe with no duration or size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlanRefusal {
    pub code: RefusalCode,
    pub suggestions: Vec<Suggestion>,
}

impl PlanRefusal {
    pub fn max_duration_ms(&self) -> Option<u64> {
        match self.code {
            RefusalCode::TooLongForLimit { max_duration_ms } => Some(max_duration_ms),
            _ => None,
        }
    }
}

impl core::fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "video cannot be planned: {:?}", self.code)
    }
}

impl std::error::Error for PlanRefusal {}

/// Duration the output will have: the trim range, clamped to the source.
pub fn output_duration_ms(probe: &VideoProbe, options: &PlanOptions) -> u64 {
    match options.trim {
        Some((start, end)) => end.min(probe.duration_ms).saturating_sub(start),
        None => probe.duration_ms,
    }
}

/// Channel count the audio planner sees: the chosen track's, or the widest
/// track when mixing; 0 when there is no audio or it is removed. A `Track`
/// index that does not exist falls back to mixing all tracks.
pub fn source_audio_channels(probe: &VideoProbe, choice: &AudioTrackChoice) -> u32 {
    match choice {
        AudioTrackChoice::Remove => 0,
        AudioTrackChoice::Track { index } => probe
            .audio
            .iter()
            .find(|t| t.index == *index)
            .map(|t| t.channels)
            .unwrap_or_else(|| mixed_channels(probe)),
        AudioTrackChoice::MixAll => mixed_channels(probe),
    }
}

fn mixed_channels(probe: &VideoProbe) -> u32 {
    probe.audio.iter().map(|t| t.channels).max().unwrap_or(0)
}

/// `(total_bps + 7) / 8 * duration + overhead`, rounded up.
fn predict_bytes(total_bps: u64, duration_ms: u64, overhead_bytes: u64) -> u64 {
    let bits = total_bps as u128 * duration_ms as u128;
    // bits per second * ms / 1000 / 8, rounded up
    bits.div_ceil(8000) as u64 + overhead_bytes
}

pub(crate) struct RungEval {
    pub overhead: u64,
    pub total: u64,
    pub audio: AudioChoice,
    pub video: u64,
    pub bpp: f64,
    pub floor: f64,
}

pub(crate) fn eval_rung(
    rung: &Rung,
    budget: &Budget,
    target: &Target,
    margin: Margin,
    duration_s: f64,
    source_channels: u32,
    choice: &AudioTrackChoice,
) -> RungEval {
    let audio_tracks_out =
        u32::from(source_channels > 0 && !matches!(choice, AudioTrackChoice::Remove));
    let overhead = container_overhead(target.container, duration_s, rung.fps, audio_tracks_out);
    let total = total_bps(budget.raw_budget_bytes, overhead, margin, duration_s);
    let audio = pick_audio(total, target.audio_codec, source_channels, choice);
    let video = total.saturating_sub(audio.bps);
    let pps = rung.pixels_per_second();
    let bpp = if pps > 0.0 { video as f64 / pps } else { 0.0 };
    let floor = floor_bpp(target.video_codec, rung.short_edge());
    RungEval {
        overhead,
        total,
        audio,
        video,
        bpp,
        floor,
    }
}

pub(crate) fn make_plan(
    rung_index: usize,
    ladder: Vec<Rung>,
    eval: &RungEval,
    target: &Target,
    encoder: EncoderKind,
    duration_ms: u64,
) -> VideoPlan {
    let rung = ladder[rung_index];
    VideoPlan {
        width: rung.width,
        height: rung.height,
        fps: rung.fps,
        video_bps: eval.video,
        audio_bps: eval.audio.bps,
        audio_channels: eval.audio.channels,
        container: target.container,
        video_codec: target.video_codec,
        audio_codec: target.audio_codec,
        two_pass: encoder.two_pass(),
        quality: quality_label(eval.bpp, eval.floor),
        predicted_bytes: predict_bytes(eval.video + eval.audio.bps, duration_ms, eval.overhead),
        rung_index,
        ladder,
        duration_ms,
        overhead_bytes: eval.overhead,
        bpp: eval.bpp,
        floor_bpp: eval.floor,
    }
}

/// Walk the ladder for a given output duration; None when no rung meets its floor.
fn try_plan(
    probe: &VideoProbe,
    budget: &Budget,
    target: &Target,
    encoder: EncoderKind,
    options: &PlanOptions,
    duration_ms: u64,
) -> Option<VideoPlan> {
    if duration_ms == 0 {
        return None;
    }
    let ladder = build_rungs(probe, &target.caps, &options.frame_rate_pref);
    let margin = Margin::for_encoder(encoder);
    let duration_s = duration_ms as f64 / 1000.0;
    let channels = source_audio_channels(probe, &options.audio);
    for (i, rung) in ladder.iter().enumerate() {
        let eval = eval_rung(
            rung,
            budget,
            target,
            margin,
            duration_s,
            channels,
            &options.audio,
        );
        if eval.total > 0 && eval.bpp >= eval.floor {
            return Some(make_plan(i, ladder, &eval, target, encoder, duration_ms));
        }
    }
    None
}

/// Plan one video (3.5.2 to 3.5.4), or refuse (3.5.5).
///
/// The overhead is evaluated per rung with that rung's frame rate, so the
/// 60 fps start rung pays for its extra frames. The budget aimed at is
/// `budget.raw_budget_bytes`.
pub fn plan(
    probe: &VideoProbe,
    budget: &Budget,
    target: &Target,
    encoder: EncoderKind,
    options: &PlanOptions,
) -> Result<VideoPlan, PlanRefusal> {
    if probe.display_w == 0 || probe.display_h == 0 {
        return Err(PlanRefusal {
            code: RefusalCode::UnsupportedInput {
                what: "video with no picture size".to_string(),
            },
            suggestions: vec![],
        });
    }
    let duration_ms = output_duration_ms(probe, options);
    if duration_ms == 0 {
        return Err(PlanRefusal {
            code: RefusalCode::UnsupportedInput {
                what: "video with no duration".to_string(),
            },
            suggestions: vec![],
        });
    }
    if let Some(p) = try_plan(probe, budget, target, encoder, options, duration_ms) {
        return Ok(p);
    }
    let max_duration_ms = max_duration_ms(probe, budget, target, encoder, options, duration_ms);
    if max_duration_ms == 0 {
        return Err(PlanRefusal {
            code: RefusalCode::BelowQualityFloor,
            suggestions: vec![],
        });
    }
    Err(PlanRefusal {
        code: RefusalCode::TooLongForLimit { max_duration_ms },
        suggestions: vec![Suggestion::Trim { max_duration_ms }],
    })
}

/// Longest output, in whole seconds, that still plans (3.5.5).
///
/// ```text
/// min_bps = floor_bpp(codec, lowest rung) * w * h * fps + audio at that total
/// max_duration_s = floor(((raw_budget - overhead_at_that_duration) * 8 * margin) / min_bps)
/// ```
///
/// solved by iterating twice on the overhead, then checked against the real
/// planner at that duration and stepped by whole seconds until the longest
/// duration that plans is found. The audio term is the rung the 15 percent
/// rule picks at the minimum total, not the lowest rung (docs/DECISIONS.md).
fn max_duration_ms(
    probe: &VideoProbe,
    budget: &Budget,
    target: &Target,
    encoder: EncoderKind,
    options: &PlanOptions,
    duration_ms: u64,
) -> u64 {
    let ladder = build_rungs(probe, &target.caps, &options.frame_rate_pref);
    let Some(lowest) = ladder.last() else {
        return 0;
    };
    let margin = Margin::for_encoder(encoder);
    let channels = source_audio_channels(probe, &options.audio);
    let has_audio = channels > 0;
    let min_video = (floor_bpp(target.video_codec, lowest.short_edge())
        * lowest.pixels_per_second())
    .ceil() as u64;
    let min_total = min_total_for_video(min_video, target.audio_codec, has_audio);
    if min_total == 0 {
        return 0;
    }
    let tracks_out = u32::from(has_audio);
    let solve = |d: f64| -> u64 {
        let overhead = container_overhead(target.container, d, lowest.fps, tracks_out);
        let usable = budget.raw_budget_bytes.saturating_sub(overhead) as f64;
        (usable * 8.0 * margin.value() / min_total as f64).floor() as u64
    };
    let d1 = solve(duration_ms as f64 / 1000.0);
    let mut d = solve(d1 as f64);
    let fits = |s: u64| try_plan(probe, budget, target, encoder, options, s * 1000).is_some();
    while d > 0 && !fits(d) {
        d -= 1;
    }
    // The planner's audio rung and overhead are coarser than the closed form;
    // walk up while the next second still plans (bounded for safety).
    let mut steps = 0;
    while steps < 600 && fits(d + 1) {
        d += 1;
        steps += 1;
    }
    d * 1000
}

/// What this video would become under another preset, for `PickPreset`
/// suggestions. None when the preset has no byte limit, no plannable video
/// format, or refuses the video.
pub fn predict_for_preset(
    probe: &VideoProbe,
    preset: &Preset,
    encoder: EncoderKind,
) -> Option<VideoPlan> {
    let limit = preset.resolve().ok()?;
    let budget = Budget::from_resolved(&limit);
    let target = Target::from_preset(preset)?;
    plan(probe, &budget, &target, encoder, &PlanOptions::default()).ok()
}

/// `PickPreset` suggestions for every preset other than `current_id` in
/// which the full video plans, with its predicted size (3.5.5). Order follows `presets`.
pub fn suggest_presets(
    probe: &VideoProbe,
    presets: &[Preset],
    encoder: EncoderKind,
    current_id: &str,
) -> Vec<Suggestion> {
    presets
        .iter()
        .filter(|p| p.id != current_id)
        .filter_map(|p| {
            predict_for_preset(probe, p, encoder).map(|plan| Suggestion::PickPreset {
                preset_id: p.id.clone(),
                predicted_bytes: plan.predicted_bytes,
            })
        })
        .collect()
}

/// The lowest audio rung's bitrate, for callers that show "at least" numbers.
pub fn lowest_audio_bps(codec: AudioCodec) -> u64 {
    video_audio_ladder(codec).last().map_or(0, |r| r.bps)
}
