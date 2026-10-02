//! The job model: Job -> Plan -> Attempts -> Result (DESIGN.md 3.1).
//! Every type crossing the host boundary derives `ts_rs::TS`; `cargo xtask types`
//! writes them to `packages/engine-client/src/generated/`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub type JobId = String;
pub type ItemId = String;
pub type ArtifactId = String;
/// Unix milliseconds.
pub type Timestamp = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Kind {
    Image,
    AnimatedImage,
    Video,
    Audio,
    Pdf,
    OfficeDoc,
    Archive,
    Text,
    Other,
}

impl Kind {
    /// The preset `limit_by_kind` bucket this kind falls in.
    pub fn limit_bucket(self) -> LimitKind {
        match self {
            Kind::Image | Kind::AnimatedImage => LimitKind::Image,
            Kind::Video => LimitKind::Video,
            Kind::Audio => LimitKind::Audio,
            Kind::Pdf | Kind::OfficeDoc => LimitKind::Document,
            Kind::Archive | Kind::Text | Kind::Other => LimitKind::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LimitKind {
    Image,
    Video,
    Audio,
    Document,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LimitScope {
    PerFile,
    PerMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResolvedKindLimit {
    pub hard_bytes: u64,
    pub raw_budget_bytes: u64,
    pub safety_bytes: u64,
}

/// A preset (or Custom) resolved into byte numbers the engine aims at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ResolvedLimit {
    /// Output (or output total) MUST be strictly less than this.
    pub hard_bytes: u64,
    /// What the engine aims at after encoding overhead and the safety margin.
    pub raw_budget_bytes: u64,
    pub safety_bytes: u64,
    pub scope: LimitScope,
    pub max_files_per_message: Option<u32>,
    pub by_kind: BTreeMap<LimitKind, ResolvedKindLimit>,
    /// Human label used in file names: "Discord", "Email", "Custom 8 MB".
    pub output_label: String,
    /// Shown in copy: "20 MB", "18 MB of attachments".
    pub stated_label: String,
}

impl ResolvedLimit {
    /// The hard limit that applies to one item of this kind.
    pub fn hard_for(&self, kind: Kind) -> u64 {
        self.by_kind
            .get(&kind.limit_bucket())
            .map(|k| k.hard_bytes)
            .unwrap_or(self.hard_bytes)
    }
    pub fn budget_for(&self, kind: Kind) -> u64 {
        self.by_kind
            .get(&kind.limit_bucket())
            .map(|k| k.raw_budget_bytes)
            .unwrap_or(self.raw_budget_bytes)
    }
    pub fn safety_for(&self, kind: Kind) -> u64 {
        self.by_kind
            .get(&kind.limit_bucket())
            .map(|k| k.safety_bytes)
            .unwrap_or(self.safety_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SmallerLevel {
    KeepQuality,
    Smallest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Goal {
    Fit {
        preset_id: String,
        limit: ResolvedLimit,
    },
    Smaller {
        level: SmallerLevel,
    },
}

impl Goal {
    pub fn limit(&self) -> Option<&ResolvedLimit> {
        match self {
            Goal::Fit { limit, .. } => Some(limit),
            Goal::Smaller { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Packaging {
    #[default]
    Auto,
    SeparateFiles,
    Zip,
    SevenZip,
    TarZst,
    TarXz,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SourceRef {
    /// Desktop: absolute path.
    Path { path: String },
    /// Web: id of a worker-side handle.
    Handle { handle_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioStreamInfo {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate_bps: Option<u64>,
    pub title: Option<String>,
}

/// Filled by inspect; everything the planners need to know about the input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub struct KindDetail {
    /// Lower-case format token: "jpeg", "png", "mp4", "docx", "pdf", "zip"...
    pub format: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub has_alpha: Option<bool>,
    pub frame_count: Option<u32>,
    pub duration_ms: Option<u64>,
    pub fps: Option<f32>,
    pub video_codec: Option<String>,
    pub audio_streams: Vec<AudioStreamInfo>,
    pub is_hdr: Option<bool>,
    pub rotation_degrees: Option<i32>,
    pub page_count: Option<u32>,
    pub entry_count: Option<u32>,
    pub encrypted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct InputItem {
    pub id: ItemId,
    pub source: SourceRef,
    /// Path relative to the dropped folder, or just the file name.
    pub rel_path: String,
    pub bytes: u64,
    pub kind: Kind,
    pub detail: KindDetail,
    /// Set when the item came from a dropped folder; the folder's display name.
    pub folder: Option<String>,
}

impl InputItem {
    pub fn file_name(&self) -> &str {
        self.rel_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&self.rel_path)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VideoFormatPref {
    #[default]
    MostCompatible,
    SmallerFiles,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AudioTrackChoice {
    #[default]
    MixAll,
    Track {
        index: u32,
    },
    Remove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FrameRatePref {
    #[default]
    Automatic,
    KeepOriginal,
    Fps30,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct VideoOptions {
    pub format: VideoFormatPref,
    /// "Faster (uses your graphics card)". Default on.
    pub faster: bool,
    pub audio: AudioTrackChoice,
    pub frame_rate: FrameRatePref,
    /// Trim range in ms, applied per item by id.
    #[ts(type = "Record<string, [number, number]>")]
    pub trims: BTreeMap<ItemId, (u64, u64)>,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            format: VideoFormatPref::MostCompatible,
            faster: true,
            audio: AudioTrackChoice::MixAll,
            frame_rate: FrameRatePref::Automatic,
            trims: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AudioFormatPref {
    #[default]
    Automatic,
    Mp3,
    Aac,
    Opus,
    Flac,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct AudioOptions {
    pub format: AudioFormatPref,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum OutputDir {
    SameAsSource,
    Folder { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct JobOptions {
    /// EXIF minus GPS. Default false.
    pub keep_photo_details: bool,
    /// GPS. Only meaningful with keep_photo_details.
    pub keep_location: bool,
    /// Default true for Fit, false for Smaller.
    pub allow_format_change: bool,
    /// "Modern formats" (WebP/AVIF) in Smaller mode.
    pub modern_formats: bool,
    pub flatten_transparency: bool,
    /// Advanced: "Limit photo size" long edge.
    pub max_long_edge: Option<u32>,
    pub keep_document_details: bool,
    pub video: VideoOptions,
    pub audio: AudioOptions,
    pub optimise_inside_archives: bool,
    pub output_dir: Option<OutputDir>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            keep_photo_details: false,
            keep_location: false,
            allow_format_change: true,
            modern_formats: false,
            flatten_transparency: false,
            max_long_edge: None,
            keep_document_details: true,
            video: VideoOptions::default(),
            audio: AudioOptions::default(),
            optimise_inside_archives: true,
            output_dir: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Job {
    pub id: JobId,
    pub items: Vec<InputItem>,
    pub goal: Goal,
    pub packaging: Packaging,
    pub options: JobOptions,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum QualityLabel {
    Okay,
    Good,
    Great,
}

impl QualityLabel {
    pub fn word(self) -> &'static str {
        match self {
            QualityLabel::Okay => "Okay",
            QualityLabel::Good => "Good",
            QualityLabel::Great => "Great",
        }
    }
}

/// The user-facing prediction for one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct Prediction {
    pub predicted_bytes: u64,
    /// True when the number came from a real encode (images), false for an estimate.
    pub exact: bool,
    /// "720p, 30 fps", "4032 × 3024 JPEG", "12 pages".
    pub summary: String,
    pub quality: Option<QualityLabel>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Strategy {
    Image {
        candidates: Vec<String>,
        max_long_edge: Option<u32>,
    },
    AnimatedImage {
        candidates: Vec<String>,
    },
    Video {
        encoder: String,
        container: String,
        width: u32,
        height: u32,
        fps: f32,
        video_bps: u64,
        audio_bps: u64,
        audio_channels: u32,
        two_pass: bool,
    },
    Audio {
        format: String,
        bitrate_bps: u64,
        channels: u32,
    },
    Pdf {
        lossless_only: bool,
    },
    Office,
    Archive {
        optimise_inside: bool,
    },
    ZipWrap,
    Copy,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ItemPlan {
    pub item_id: ItemId,
    /// None in Smaller mode.
    pub budget_bytes: Option<u64>,
    pub strategy: Strategy,
    pub prediction: Prediction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum PackagingPlan {
    SeparateFiles,
    Archive {
        format: Packaging,
        file_name: String,
        overhead_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum PlanVerdict {
    WillFit {
        quality: QualityLabel,
    },
    /// Hardware encoders; shown as "about".
    Uncertain {
        quality: QualityLabel,
    },
    /// Shown before any encoding happens.
    CannotFit {
        refusal: Refusal,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Plan {
    pub job_id: JobId,
    pub items: Vec<ItemPlan>,
    pub packaging: PackagingPlan,
    pub predicted_total_bytes: u64,
    pub verdict: PlanVerdict,
    /// Headline sentence for step 3, built by `copy::plan_headline`.
    pub headline: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum AttemptVerdict {
    Fits,
    Over { by_bytes: u64 },
    BelowFloor,
    Error { message: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Attempt {
    pub n: u32,
    pub item_id: ItemId,
    /// "mozjpeg", "libwebp-lossless", "h264_nvenc", "libvpx-vp9-2pass"...
    pub encoder: String,
    #[ts(type = "Record<string, unknown>")]
    pub params: serde_json::Value,
    pub output_bytes: Option<u64>,
    /// SSIMULACRA2 where computed.
    pub score: Option<f32>,
    pub elapsed_ms: u64,
    pub verdict: AttemptVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum OutputLocation {
    Path { path: String },
    Opfs { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct VerificationReport {
    pub size_ok: bool,
    pub decodes: bool,
    pub checks: Vec<String>,
    pub failures: Vec<String>,
}

impl VerificationReport {
    pub fn passed(&self) -> bool {
        self.size_ok && self.decodes && self.failures.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Artifact {
    pub id: ArtifactId,
    pub item_id: ItemId,
    pub location: OutputLocation,
    pub file_name: String,
    pub bytes: u64,
    /// "jpeg", "mp4", "zip" ...
    pub format: String,
    pub summary: String,
    pub quality: Option<QualityLabel>,
    pub verification: VerificationReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum RefusalCode {
    TooLongForLimit {
        max_duration_ms: u64,
    },
    BelowQualityFloor,
    CannotShrinkType,
    TooManyFilesForMessage {
        max: u32,
    },
    TotalTooBig,
    NeedsFfmpeg,
    BrowserLacksCodec {
        codec: String,
    },
    /// Desktop: no usable encoder for the codec the destination requires (DESIGN 3.5.6 step 5).
    NoEncoder {
        codec: String,
    },
    UnsupportedInput {
        what: String,
    },
    Encrypted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Suggestion {
    Trim {
        max_duration_ms: u64,
    },
    PickPreset {
        preset_id: String,
        predicted_bytes: u64,
    },
    SplitIntoMessages {
        groups: Vec<Vec<ItemId>>,
    },
    RemoveFiles {
        item_ids: Vec<ItemId>,
    },
    InstallFfmpeg,
    UseDesktopApp,
    UseOtherBrowser {
        browser: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Refusal {
    pub code: RefusalCode,
    /// Final user-facing sentence, built by `copy.rs`.
    pub message: String,
    /// Best we reached, for the explanation only.
    pub smallest_bytes: Option<u64>,
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Failure {
    /// Stable code for logs: "damaged_input", "encoder_crash", "disk_full", "not_writable", "source_vanished", "over_after_retries".
    pub code: String,
    pub message: String,
    pub closest_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ItemOutcome {
    Fitted {
        artifact: Artifact,
    },
    /// The original already fits and nothing we tried was meaningfully smaller.
    KeptOriginal {
        artifact: Artifact,
    },
    Refused {
        refusal: Refusal,
    },
    Failed {
        failure: Failure,
    },
    Cancelled,
}

impl ItemOutcome {
    pub fn counts_for_allowance(&self) -> bool {
        matches!(self, ItemOutcome::Fitted { .. })
    }
    pub fn artifact(&self) -> Option<&Artifact> {
        match self {
            ItemOutcome::Fitted { artifact } | ItemOutcome::KeptOriginal { artifact } => {
                Some(artifact)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobVerdict {
    AllFit,
    SomeFit,
    NoneFit,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobSummary {
    pub job_id: JobId,
    pub outcomes: Vec<(ItemId, ItemOutcome)>,
    pub packaged: Option<Artifact>,
    pub input_bytes: u64,
    pub total_bytes: u64,
    pub verdict: JobVerdict,
    /// Headline for the result card, built by `copy::result_headline`.
    pub headline: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobState {
    Created,
    Inspecting,
    Ready,
    Planned,
    Running,
    Verifying,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ItemState {
    Queued,
    Inspecting,
    Planning,
    Encoding { attempt: u32 },
    Verifying { attempt: u32 },
    Retry { attempt: u32 },
    Fitted,
    KeptOriginal,
    Refused,
    Failed,
    Cancelled,
}

/// What a host can do (DESIGN.md 2.7). Filled by each host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct Capabilities {
    pub host: String,
    pub ffmpeg: Option<FfmpegCapabilities>,
    pub web: Option<WebCapabilities>,
    pub can_copy_files: bool,
    pub can_drag_out: bool,
    pub can_reveal: bool,
    pub can_choose_folder: bool,
    pub heic_input: bool,
    pub avif_input: bool,
    pub opus_encode: bool,
    pub mp3_encode: bool,
    pub aac_encode: bool,
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct FfmpegCapabilities {
    pub version: String,
    pub path: String,
    pub user_supplied: bool,
    pub working_encoders: Vec<String>,
    pub has_libx264: bool,
    pub has_libvpx_vp9: bool,
    pub has_aac: bool,
    pub has_libmp3lame: bool,
    pub has_libopus: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[serde(default)]
#[ts(export)]
pub struct WebCapabilities {
    pub cross_origin_isolated: bool,
    pub webcodecs: bool,
    pub h264_encode: bool,
    pub vp9_encode: bool,
    pub aac_encode: bool,
    pub opus_encode: bool,
    pub hdr_verified: bool,
    pub directory_picker: bool,
    pub opfs: bool,
    pub browser: String,
}
