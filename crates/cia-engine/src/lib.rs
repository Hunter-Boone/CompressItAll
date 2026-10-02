//! The Smidge orchestrator (DESIGN.md 3.1 to 3.3, 3.9 to 3.12). Hosts give it
//! an `InputReader` (how to get bytes), an `OutputSink` (where results go),
//! optional backends for video (FFmpeg natively, WebCodecs on the web), and an
//! `EventSink`. The engine does the rest: inspect, plan, run, verify, name,
//! package, log. It never touches the UI.

pub mod detect;
pub mod inspect;
pub mod log;
pub mod output;
pub mod plan;
pub mod planners;
pub mod run;
#[cfg(feature = "ffmpeg")]
pub mod video_ffmpeg;
#[cfg(feature = "ffmpeg")]
pub use cia_ffmpeg;

use cia_core::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use cia_core;
pub use output::{MemorySink, OutputDest, OutputSink};
pub use plan::PlanRequest;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Io(String),
    #[error("damaged_input: {0}")]
    Damaged(String),
    #[error("unsupported_input: {0}")]
    Unsupported(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

/// Reads input bytes for a `SourceRef`. Desktop: the filesystem. Web: worker-side File handles.
pub trait InputReader: Send + Sync {
    fn read(&self, source: &SourceRef) -> Result<Vec<u8>, EngineError>;
    /// Only the first `n` bytes, for detection; defaults to a full read.
    fn read_head(&self, source: &SourceRef, n: usize) -> Result<Vec<u8>, EngineError> {
        let mut b = self.read(source)?;
        b.truncate(n);
        Ok(b)
    }
    fn len(&self, source: &SourceRef) -> Result<u64, EngineError>;
    /// Directory of the source for "save next to the original"; None on the web.
    fn parent_dir(&self, _source: &SourceRef) -> Option<String> {
        None
    }
}

/// Native video work (probe + transcode + verify + frame extraction). Provided by cia-ffmpeg on
/// desktop; the web host runs WebCodecs in JS and reports results through the same trait.
pub trait VideoBackend: Send + Sync {
    fn probe(&self, source: &SourceRef) -> Result<cia_video_plan::VideoProbe, EngineError>;
    /// Encode per `plan` into `dest`; returns the written bytes count, the final plan used and the attempt log.
    #[allow(clippy::too_many_arguments)]
    fn transcode(
        &self,
        source: &SourceRef,
        probe: &cia_video_plan::VideoProbe,
        budget: Option<cia_video_plan::Budget>,
        target: cia_video_plan::Target,
        options: &cia_video_plan::PlanOptions,
        allowed: &[cia_core::presets::VideoFormat],
        faster: bool,
        dest: &OutputDest,
        progress: &dyn Fn(f32, Option<u64>),
        cancel: &CancelToken,
    ) -> Result<VideoTranscodeResult, EngineError>;
    fn encoder_kind(&self, faster: bool) -> cia_video_plan::EncoderKind;
    fn capabilities(&self) -> Option<FfmpegCapabilities>;
    /// Host capability check before planning (DESIGN.md 3.5.10 step 1). Runs
    /// after the target has been picked from the preset and may switch it (a
    /// browser without an H.264 encoder gets VP9 WebM when the preset allows
    /// it) or refuse the item with a code and suggestions. Native FFmpeg has
    /// its own encoder chain and keeps the default.
    fn check_support(
        &self,
        _source: &SourceRef,
        _probe: &cia_video_plan::VideoProbe,
        target: cia_video_plan::Target,
        _allowed: &[cia_core::presets::VideoFormat],
        _options: &cia_video_plan::PlanOptions,
    ) -> Result<cia_video_plan::Target, (RefusalCode, Vec<Suggestion>)> {
        Ok(target)
    }
    /// Duration and channel count of an audio-only file the pure-Rust decoders cannot read.
    fn probe_audio(&self, source: &SourceRef) -> Result<(u64, u16, u32), EngineError> {
        self.probe(source).map(|p| {
            (
                p.duration_ms,
                p.audio.first().map(|a| a.channels as u16).unwrap_or(2),
                p.audio.first().map(|a| a.sample_rate).unwrap_or(44_100),
            )
        })
    }
    /// Encode audio to MP3 or AAC (formats the pure-Rust crates cannot produce). `format` is "mp3" or "m4a_aac".
    fn encode_audio(
        &self,
        _source: &SourceRef,
        _format: &str,
        _bitrate_bps: u32,
        _channels: u16,
        _sample_rate: u32,
        _trim: Option<(u64, u64)>,
    ) -> Result<Vec<u8>, EngineError> {
        Err(EngineError::Unsupported("audio encoder".into()))
    }
}

/// What a host that transcodes video outside the engine (the web app's
/// WebCodecs worker) needs for one item: the same budget, target and options
/// `run_video` would hand the backend. See [`Engine::video_work`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VideoWork {
    pub item_id: String,
    pub source: SourceRef,
    /// Always concrete: Smaller mode aims at 60 percent of the source, as the native backend does.
    pub budget: cia_video_plan::Budget,
    pub target: cia_video_plan::Target,
    pub options: cia_video_plan::PlanOptions,
    pub faster: bool,
    pub allowed: Vec<cia_core::presets::VideoFormat>,
    /// `hard_bytes` for the verification size check (None in Smaller mode).
    pub hard_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct VideoTranscodeResult {
    pub location: OutputLocation,
    pub file_name: String,
    pub bytes: u64,
    /// None when the original was kept.
    pub plan: Option<cia_video_plan::VideoPlan>,
    pub attempts: Vec<Attempt>,
    pub verification: VerificationReport,
    /// True when the source was copied/remuxed rather than re-encoded.
    pub kept_original: bool,
}

/// Cooperative cancellation. `cancel()` flips a flag; hosts that cannot call
/// into the engine while it runs (a Web Worker executing a synchronous `run`)
/// attach a poll closure with [`CancelToken::with_poll`] that reads a flag the
/// main thread writes (an `Int32Array` over a `SharedArrayBuffer`).
#[derive(Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
    poll: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// A token that is also cancelled whenever `poll()` returns true.
    pub fn with_poll(poll: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            poll: Some(poll),
        }
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst) || self.poll.as_ref().is_some_and(|p| p())
    }
}

/// Everything the engine needs from its host.
pub struct Engine {
    pub reader: Arc<dyn InputReader>,
    pub sink: Arc<dyn OutputSink>,
    pub video: Option<Arc<dyn VideoBackend>>,
    pub caps: Capabilities,
    /// Where temporary files go (desktop); unused on the web.
    pub temp_dir: Option<String>,
    /// Max parallel items (rayon on native). 1 on wasm.
    pub parallelism: usize,
    /// Encoded results from the last preview, reused by `run` so images are not encoded twice.
    pub preview_cache: std::sync::Mutex<std::collections::HashMap<String, planners::Encoded>>,
}

impl Engine {
    pub fn new(
        reader: Arc<dyn InputReader>,
        sink: Arc<dyn OutputSink>,
        caps: Capabilities,
    ) -> Self {
        Self {
            reader,
            sink,
            video: None,
            caps,
            temp_dir: None,
            parallelism: default_parallelism(),
            preview_cache: Default::default(),
        }
    }
    pub fn with_video(mut self, video: Arc<dyn VideoBackend>) -> Self {
        self.caps.ffmpeg = video.capabilities();
        self.caps.video = true;
        self.video = Some(video);
        self
    }
    pub fn can_video(&self) -> bool {
        self.video.is_some()
    }
}

fn default_parallelism() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        1
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 8)
    }
}

/// Current time in unix ms (0 on hosts without a clock; the host may override via `now_ms`).
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_now_ms()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[cfg(target_arch = "wasm32")]
fn js_now_ms() -> u64 {
    // Set by the wasm host at start-up; avoids a js-sys dependency here.
    NOW_MS_HOOK.with(|h| h.get().map(|f| f()).unwrap_or(0))
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    pub static NOW_MS_HOOK: std::cell::Cell<Option<fn() -> u64>> = const { std::cell::Cell::new(None) };
}
