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
        faster: bool,
        dest: &OutputDest,
        progress: &dyn Fn(f32, Option<u64>),
        cancel: &CancelToken,
    ) -> Result<VideoTranscodeResult, EngineError>;
    fn encoder_kind(&self, faster: bool) -> cia_video_plan::EncoderKind;
    fn capabilities(&self) -> Option<FfmpegCapabilities>;
}

#[derive(Debug, Clone)]
pub struct VideoTranscodeResult {
    pub location: OutputLocation,
    pub file_name: String,
    pub bytes: u64,
    pub plan: cia_video_plan::VideoPlan,
    pub attempts: Vec<Attempt>,
    pub verification: VerificationReport,
    /// True when the source was copied/remuxed rather than re-encoded.
    pub kept_original: bool,
}

#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
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
}

impl Engine {
    pub fn new(reader: Arc<dyn InputReader>, sink: Arc<dyn OutputSink>, caps: Capabilities) -> Self {
        Self { reader, sink, video: None, caps, temp_dir: None, parallelism: default_parallelism() }
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
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8)
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
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
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
