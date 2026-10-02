//! The Smidge engine for the web app (DESIGN.md 2.2, 2.5, 3.10, 3.12).
//!
//! One instance of this module lives inside each Web Worker
//! (`apps/web/src/workers/engine.worker.ts`). The worker owns every byte
//! buffer: it reads `File` objects and hands the bytes to [`add_file`]; the
//! engine's `InputReader` serves them back from a thread-local map. Outputs
//! land in another thread-local map keyed by their OPFS path
//! (`/jobs/<job_id>/<file name>`) and the worker moves them to OPFS with
//! [`take_output`].
//!
//! Serialisation: everything crossing the boundary is a `cia-core` type
//! serialised with serde-wasm-bindgen. `u64` fields become JS `BigInt`, maps
//! become plain objects and `None` becomes `null`, which is what the ts-rs
//! bindings in `packages/engine-client/src/generated` describe.
//!
//! Cancellation: `run` is synchronous inside the worker, so the main thread
//! cannot call into it. Instead it writes 1 into an `Int32Array` over a
//! `SharedArrayBuffer` and the engine's `CancelToken` polls that cell.

use cia_core::events::{EngineEvent, EventSink};
use cia_core::*;
use cia_engine::inspect::InputSpec;
use cia_engine::{
    CancelToken, Engine, EngineError, InputReader, OutputDest, OutputSink, VideoBackend,
    VideoTranscodeResult,
};
use cia_video_plan::{
    AudioCodec, Budget, Container, Decision, EncoderKind, PlanOptions, PlanRefusal, Target,
    VideoCodec, VideoPlan, VideoProbe,
};
use cia_wasm_libc as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

/// handle_id -> (file name, bytes)
type FileMap = HashMap<String, (String, Rc<Vec<u8>>)>;
/// `(probe, VideoDecoder takes the codec)`, or `Err` for a file mediabunny could not read.
type ProbeEntry = Result<(VideoProbe, bool), ()>;

thread_local! {
    static FILES: RefCell<FileMap> = RefCell::new(HashMap::new());
    /// OPFS path -> bytes written by the engine and not yet taken by the worker.
    static OUTPUTS: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
    static CAPS: RefCell<Option<Capabilities>> = const { RefCell::new(None) };
    /// job_id -> job log JSON (kept for the session; DESIGN.md 3.12).
    static LOGS: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
    /// handle_id -> what the JS probe (mediabunny) found: the probe and whether
    /// this browser's `VideoDecoder` takes the codec; `Err` when the file could
    /// not be read (damaged).
    static PROBES: RefCell<HashMap<String, ProbeEntry>> = RefCell::new(HashMap::new());
    /// handle_id -> the finished WebCodecs transcode for the job about to run.
    static VIDEO_RESULTS: RefCell<HashMap<String, VideoResultIn>> = RefCell::new(HashMap::new());
    /// handle_id -> output bytes of that transcode, taken once by the sink.
    static VIDEO_BYTES: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn serializer() -> serde_wasm_bindgen::Serializer {
    serde_wasm_bindgen::Serializer::new()
        .serialize_large_number_types_as_bigints(true)
        .serialize_maps_as_objects(true)
        .serialize_missing_as_null(true)
}

fn to_js<T: Serialize>(v: &T) -> Result<JsValue, JsError> {
    v.serialize(&serializer()).map_err(js_err)
}

fn from_js<T: DeserializeOwned>(v: JsValue) -> Result<T, JsError> {
    serde_wasm_bindgen::from_value(v).map_err(js_err)
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

/// Call once per worker before anything else: panic messages go to the
/// console and the engine gets a clock.
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();
    // The hook only exists on wasm32; this crate also compiles natively for the workspace lint.
    #[cfg(target_arch = "wasm32")]
    cia_engine::NOW_MS_HOOK.with(|h| h.set(Some(now_ms)));
}

/// Engine version: crate version plus the git short SHA baked in by `tools/build-wasm.sh`.
#[wasm_bindgen]
pub fn version() -> String {
    match option_env!("CIA_GIT_SHA") {
        Some(sha) if !sha.is_empty() => format!("{}+{}", env!("CARGO_PKG_VERSION"), sha),
        _ => env!("CARGO_PKG_VERSION").to_string(),
    }
}

// ---------------------------------------------------------------------------
// Capabilities

/// Build the web host's `Capabilities` from what the JS side probed
/// (`WebCapabilities`) and remember them for the engine.
///
/// Opus and video stay off until the WebCodecs pipelines exist
/// (DESIGN.md 3.5.10, 3.6): the probe results are still reported in `web.*`
/// so the UI can explain what this browser could do.
#[wasm_bindgen]
pub fn capabilities(web: JsValue) -> Result<JsValue, JsError> {
    let web: WebCapabilities = from_js(web)?;
    let caps = Capabilities {
        host: "web".into(),
        ffmpeg: None,
        can_copy_files: false,
        can_drag_out: false,
        can_reveal: false,
        can_choose_folder: web.directory_picker,
        heic_input: false,
        avif_input: false,
        opus_encode: false,
        mp3_encode: false,
        aac_encode: false,
        // 3.5.10: the pipeline needs one of the two encoders; the UI shows the 4.7 banner otherwise.
        video: web.webcodecs && (web.h264_encode || web.vp9_encode),
        web: Some(web),
    };
    CAPS.with(|c| *c.borrow_mut() = Some(caps.clone()));
    ENGINE.with(|e| *e.borrow_mut() = None);
    to_js(&caps)
}

fn engine() -> Rc<Engine> {
    ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        if let Some(eng) = e.as_ref() {
            return eng.clone();
        }
        let caps = CAPS
            .with(|c| c.borrow().clone())
            .unwrap_or_else(|| Capabilities {
                host: "web".into(),
                ..Default::default()
            });
        let video = caps.video;
        let mut eng = Engine::new(Arc::new(WasmReader), Arc::new(WasmSink), caps)
            .with_video(Arc::new(WasmVideoBackend));
        // `with_video` assumes a backend means video works; here the browser decides.
        eng.caps.video = video;
        let eng = Rc::new(eng);
        *e = Some(eng.clone());
        eng
    })
}

// ---------------------------------------------------------------------------
// Inputs

/// Store a file's bytes under `id` (the `handle_id` of its `SourceRef`).
#[wasm_bindgen]
pub fn add_file(id: &str, name: &str, bytes: js_sys::Uint8Array) {
    let v = bytes.to_vec();
    FILES.with(|f| {
        f.borrow_mut()
            .insert(id.to_string(), (name.to_string(), Rc::new(v)))
    });
}

#[wasm_bindgen]
pub fn has_file(id: &str) -> bool {
    FILES.with(|f| f.borrow().contains_key(id))
}

#[wasm_bindgen]
pub fn remove_file(id: &str) {
    FILES.with(|f| f.borrow_mut().remove(id));
    PROBES.with(|p| p.borrow_mut().remove(id));
    VIDEO_RESULTS.with(|r| r.borrow_mut().remove(id));
    VIDEO_BYTES.with(|b| b.borrow_mut().remove(id));
    // The preview cache holds encodes keyed by item id; drop it so memory follows the file.
    ENGINE.with(|e| {
        if let Some(eng) = e.borrow().as_ref() {
            eng.preview_cache.lock().unwrap().clear();
        }
    });
}

#[wasm_bindgen]
pub fn clear_files() {
    FILES.with(|f| f.borrow_mut().clear());
    PROBES.with(|p| p.borrow_mut().clear());
    clear_video_results();
    ENGINE.with(|e| {
        if let Some(eng) = e.borrow().as_ref() {
            eng.preview_cache.lock().unwrap().clear();
        }
    });
}

/// Bytes held for inputs, for the pool's memory accounting.
#[wasm_bindgen]
pub fn held_bytes() -> f64 {
    let files: usize = FILES.with(|f| f.borrow().values().map(|(_, b)| b.len()).sum());
    let outs: usize = OUTPUTS.with(|o| o.borrow().values().map(Vec::len).sum());
    let video: usize = VIDEO_BYTES.with(|b| b.borrow().values().map(Vec::len).sum());
    (files + outs + video) as f64
}

struct WasmReader;

impl WasmReader {
    fn get(source: &SourceRef) -> Result<Rc<Vec<u8>>, EngineError> {
        match source {
            SourceRef::Handle { handle_id } => FILES.with(|f| {
                f.borrow()
                    .get(handle_id)
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| EngineError::Io("source_vanished".into()))
            }),
            SourceRef::Path { .. } => Err(EngineError::Unsupported("path on the web".into())),
        }
    }
}

impl InputReader for WasmReader {
    fn read(&self, source: &SourceRef) -> Result<Vec<u8>, EngineError> {
        Self::get(source).map(|b| b.as_ref().clone())
    }
    fn read_head(&self, source: &SourceRef, n: usize) -> Result<Vec<u8>, EngineError> {
        Self::get(source).map(|b| b[..b.len().min(n)].to_vec())
    }
    fn len(&self, source: &SourceRef) -> Result<u64, EngineError> {
        Self::get(source).map(|b| b.len() as u64)
    }
}

#[derive(Deserialize)]
struct SpecIn {
    id: String,
    rel_path: String,
    #[serde(default)]
    folder: Option<String>,
}

/// `[{id, rel_path, folder}]` -> `InputItem[]`. Bytes must have been added first.
#[wasm_bindgen]
pub fn inspect(specs: JsValue) -> Result<JsValue, JsError> {
    let specs: Vec<SpecIn> = from_js(specs)?;
    let specs: Vec<InputSpec> = specs
        .into_iter()
        .map(|s| InputSpec {
            source: SourceRef::Handle { handle_id: s.id },
            rel_path: s.rel_path,
            folder: s.folder,
        })
        .collect();
    let items = engine().inspect(&specs);
    to_js(&items)
}

// ---------------------------------------------------------------------------
// Outputs

/// In-memory sink keyed by path. `dir` is "" for the job root (the web reader
/// has no parent directory) or a folder the engine created with `make_dir`.
struct WasmSink;

impl WasmSink {
    fn key(dir: &str, name: &str) -> String {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", dir.trim_end_matches('/'), name)
        }
    }
}

fn loc_path(location: &OutputLocation) -> &str {
    let (OutputLocation::Opfs { path } | OutputLocation::Path { path }) = location;
    path
}

/// Sink key under which a finished WebCodecs transcode is readable (handle id after the colon).
const VIDEO_KEY: &str = "video:";

impl OutputSink for WasmSink {
    fn exists(&self, dir: &str, file_name: &str) -> bool {
        OUTPUTS.with(|o| o.borrow().contains_key(&Self::key(dir, file_name)))
    }
    fn write(&self, dest: &OutputDest, bytes: &[u8]) -> Result<OutputLocation, EngineError> {
        let key = Self::key(&dest.dir, &dest.file_name);
        OUTPUTS.with(|o| {
            let mut o = o.borrow_mut();
            if o.contains_key(&key) {
                return Err(EngineError::Io(format!("{key} already exists")));
            }
            o.insert(key.clone(), bytes.to_vec());
            Ok(OutputLocation::Opfs { path: key })
        })
    }
    fn len(&self, location: &OutputLocation) -> Result<u64, EngineError> {
        let path = loc_path(location);
        if let Some(id) = path.strip_prefix(VIDEO_KEY) {
            return VIDEO_BYTES.with(|b| {
                b.borrow()
                    .get(id)
                    .map(|v| v.len() as u64)
                    .ok_or_else(|| EngineError::Io(format!("{path} missing")))
            });
        }
        OUTPUTS.with(|o| {
            o.borrow()
                .get(path)
                .map(|b| b.len() as u64)
                .ok_or_else(|| EngineError::Io(format!("{path} missing")))
        })
    }
    fn read(&self, location: &OutputLocation) -> Result<Vec<u8>, EngineError> {
        let path = loc_path(location);
        if let Some(id) = path.strip_prefix(VIDEO_KEY) {
            // The runner reads once and removes; hand the buffer over instead of copying it
            // (a second copy of a large video would double the worker's memory).
            return VIDEO_BYTES.with(|b| {
                b.borrow_mut()
                    .remove(id)
                    .ok_or_else(|| EngineError::Io(format!("{path} missing")))
            });
        }
        OUTPUTS.with(|o| {
            o.borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| EngineError::Io(format!("{path} missing")))
        })
    }
    fn remove(&self, location: &OutputLocation) {
        let path = loc_path(location);
        if let Some(id) = path.strip_prefix(VIDEO_KEY) {
            VIDEO_BYTES.with(|b| b.borrow_mut().remove(id));
            return;
        }
        OUTPUTS.with(|o| o.borrow_mut().remove(path));
    }
    fn make_dir(&self, dir: &str, sub: &str) -> Result<String, EngineError> {
        Ok(Self::key(dir, sub))
    }
}

fn job_prefix(job_id: &str) -> String {
    format!("/jobs/{job_id}")
}

/// Rewrite a sink-relative location into its OPFS path. Paths that are not in
/// the output map (archive entries that live inside the zip, `source:<id>`
/// kept originals) are left alone.
fn prefix_location(location: &mut OutputLocation, prefix: &str) {
    if let OutputLocation::Opfs { path } = location {
        if path.starts_with('/') || path.starts_with("source:") {
            return;
        }
        if OUTPUTS.with(|o| o.borrow().contains_key(path.as_str())) {
            *path = format!("{prefix}/{path}");
        }
    }
}

fn prefix_outcome(outcome: &mut ItemOutcome, prefix: &str) {
    if let ItemOutcome::Fitted { artifact } | ItemOutcome::KeptOriginal { artifact } = outcome {
        prefix_location(&mut artifact.location, prefix);
    }
}

fn prefix_summary(summary: &mut JobSummary, prefix: &str) {
    for (_, o) in summary.outcomes.iter_mut() {
        prefix_outcome(o, prefix);
    }
    if let Some(p) = summary.packaged.as_mut() {
        prefix_location(&mut p.location, prefix);
    }
}

/// After a run: move every output under `/jobs/<job_id>/` so the paths the
/// summary reports are the ones `take_output` understands.
fn prefix_outputs(prefix: &str) {
    OUTPUTS.with(|o| {
        let mut o = o.borrow_mut();
        let old = std::mem::take(&mut *o);
        for (k, v) in old {
            let key = if k.starts_with('/') {
                k
            } else {
                format!("{prefix}/{k}")
            };
            o.insert(key, v);
        }
    });
}

/// Paths of outputs waiting to be moved to OPFS.
#[wasm_bindgen]
pub fn list_outputs() -> Vec<String> {
    OUTPUTS.with(|o| o.borrow().keys().cloned().collect())
}

/// Remove an output from the engine's memory and return its bytes.
#[wasm_bindgen]
pub fn take_output(path: &str) -> Option<js_sys::Uint8Array> {
    OUTPUTS.with(|o| o.borrow_mut().remove(path)).map(|b| {
        let arr = js_sys::Uint8Array::new_with_length(b.len() as u32);
        arr.copy_from(&b);
        arr
    })
}

/// Drop whatever outputs are still held (cancelled or failed jobs).
#[wasm_bindgen]
pub fn drop_outputs() {
    OUTPUTS.with(|o| o.borrow_mut().clear());
}

// ---------------------------------------------------------------------------
// Preview and run

#[wasm_bindgen]
pub fn preview(req: JsValue) -> Result<JsValue, JsError> {
    let req: cia_engine::PlanRequest = from_js(req)?;
    let plan = engine()
        .preview(&req, &CancelToken::new())
        .map_err(js_err)?;
    to_js(&plan)
}

/// wasm32 is single-threaded; JS values never cross a thread, so the Send +
/// Sync the engine traits ask for is vacuous here.
struct SingleThread<T>(T);
unsafe impl<T> Send for SingleThread<T> {}
unsafe impl<T> Sync for SingleThread<T> {}

struct JsEventSink {
    on_event: SingleThread<js_sys::Function>,
    job_id: RefCell<Option<String>>,
    /// item_id -> last Progress emit (ms), for the 10 per second throttle (3.12).
    last_progress: RefCell<HashMap<String, u64>>,
}
unsafe impl Send for JsEventSink {}
unsafe impl Sync for JsEventSink {}

impl EventSink for JsEventSink {
    fn emit(&self, mut event: EngineEvent) {
        match &mut event {
            EngineEvent::JobState { job_id, .. } => {
                *self.job_id.borrow_mut() = Some(job_id.clone());
            }
            EngineEvent::Progress { item_id, .. } => {
                let now = now_ms();
                let mut last = self.last_progress.borrow_mut();
                if last
                    .get(item_id)
                    .is_some_and(|t| now.saturating_sub(*t) < 100)
                {
                    return;
                }
                last.insert(item_id.clone(), now);
            }
            EngineEvent::ItemDone {
                job_id, outcome, ..
            } => {
                prefix_outcome(outcome, &job_prefix(job_id));
            }
            EngineEvent::JobDone { job_id, summary } => {
                prefix_summary(summary, &job_prefix(job_id));
            }
            _ => {}
        }
        if let Ok(v) = to_js(&event) {
            let _ = self.on_event.0.call1(&JsValue::NULL, &v);
        }
    }
}

/// Run a job. `on_event` receives every `EngineEvent`; `cancel_flag`, when
/// given, is an `Int32Array` over a `SharedArrayBuffer` whose first element
/// the main thread sets to a non-zero value to cancel.
#[wasm_bindgen]
pub fn run(
    req: JsValue,
    on_event: js_sys::Function,
    cancel_flag: Option<js_sys::Int32Array>,
) -> Result<JsValue, JsError> {
    run_job(&cia_core::new_id(), req, on_event, cancel_flag)
}

/// [`run`] with a job id the host chose, so it can tag the events of the
/// video transcodes it ran ahead of the engine with the same id.
#[wasm_bindgen]
pub fn run_job(
    job_id: &str,
    req: JsValue,
    on_event: js_sys::Function,
    cancel_flag: Option<js_sys::Int32Array>,
) -> Result<JsValue, JsError> {
    let req: cia_engine::PlanRequest = from_js(req)?;
    let cancel = match cancel_flag {
        Some(flag) => {
            let flag = SingleThread(flag);
            CancelToken::with_poll(Arc::new(move || {
                // Borrow the wrapper as a whole so the closure captures the `Send` wrapper, not the field.
                let flag: &SingleThread<js_sys::Int32Array> = &flag;
                js_sys::Atomics::load(&flag.0, 0).unwrap_or(0) != 0
            }))
        }
        None => CancelToken::new(),
    };
    let sink = JsEventSink {
        on_event: SingleThread(on_event),
        job_id: RefCell::new(None),
        last_progress: RefCell::new(HashMap::new()),
    };
    drop_outputs();
    let (mut summary, log) = engine().run_job(job_id, &req, &sink, &cancel);
    // Whatever the runner did not consume (cancelled before the item) is not an output.
    clear_video_results();
    let prefix = job_prefix(&summary.job_id);
    // The summary from `run` was built before the sink rewrote paths; redo it against the map.
    prefix_summary(&mut summary, &prefix);
    prefix_outputs(&prefix);
    LOGS.with(|l| {
        let mut l = l.borrow_mut();
        l.insert(summary.job_id.clone(), log.to_json());
        while l.len() > 10 {
            let first = l.keys().next().cloned();
            if let Some(k) = first {
                l.remove(&k);
            }
        }
    });
    to_js(&summary)
}

/// JSON job log for a job this worker ran (last 10 kept).
#[wasm_bindgen]
pub fn job_log(job_id: &str) -> Option<String> {
    LOGS.with(|l| l.borrow().get(job_id).cloned())
}

// ---------------------------------------------------------------------------
// Video (DESIGN.md 3.5.10, docs/DECISIONS.md "web video split")
//
// WebCodecs is asynchronous and lives in JS, while the engine's runner is
// synchronous. The host therefore probes every video with mediabunny and
// registers the probe here (`set_video_probe`), and before `run_job` it
// transcodes each video item the engine would have handed to a backend
// (`video_work`) and registers the finished result (`set_video_result`).
// `WasmVideoBackend` serves both back to the engine, which keeps planning,
// naming, verification bookkeeping, packaging and the summary in Rust.

/// A finished (or failed) WebCodecs transcode, as the video worker reports it.
#[derive(Debug, Clone, Deserialize)]
struct VideoResultIn {
    #[serde(default)]
    kept_original: bool,
    #[serde(default)]
    plan: Option<VideoPlan>,
    #[serde(default)]
    attempts: Vec<Attempt>,
    #[serde(default)]
    verification: Option<VerificationReport>,
    /// "cancelled", "damaged", "refused", "over" or a failure code; None when the encode fit.
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    closest_bytes: Option<u64>,
    #[serde(default)]
    max_duration_ms: Option<u64>,
}

struct WasmVideoBackend;

fn handle_of(source: &SourceRef) -> Option<&str> {
    match source {
        SourceRef::Handle { handle_id } => Some(handle_id),
        SourceRef::Path { .. } => None,
    }
}

impl VideoBackend for WasmVideoBackend {
    fn probe(&self, source: &SourceRef) -> Result<VideoProbe, EngineError> {
        let id = handle_of(source).ok_or_else(|| EngineError::Unsupported("path".into()))?;
        PROBES.with(|p| match p.borrow().get(id) {
            Some(Ok((probe, _))) => Ok(probe.clone()),
            Some(Err(())) => Err(EngineError::Damaged("video".into())),
            None => Err(EngineError::Unsupported("not probed yet".into())),
        })
    }

    fn check_support(
        &self,
        source: &SourceRef,
        probe: &VideoProbe,
        target: Target,
        allowed: &[cia_core::presets::VideoFormat],
        options: &PlanOptions,
    ) -> Result<Target, (RefusalCode, Vec<Suggestion>)> {
        let web = CAPS
            .with(|c| c.borrow().as_ref().and_then(|c| c.web.clone()))
            .unwrap_or_default();
        let browser = || {
            vec![
                Suggestion::UseOtherBrowser {
                    browser: "Chrome, Edge or Safari".into(),
                },
                Suggestion::UseDesktopApp,
            ]
        };
        let can_decode = handle_of(source)
            .and_then(|id| {
                PROBES.with(|p| {
                    p.borrow()
                        .get(id)
                        .and_then(|r| r.as_ref().ok().map(|(_, d)| *d))
                })
            })
            .unwrap_or(false);
        if !can_decode {
            return Err((
                RefusalCode::BrowserLacksCodec {
                    codec: probe.video_codec.clone(),
                },
                browser(),
            ));
        }
        // 3.5.10 step 4: tone-mapping only on browsers the HDR fixture has verified.
        if probe.is_hdr && !web.hdr_verified {
            return Err((
                RefusalCode::UnsupportedInput { what: "hdr".into() },
                vec![Suggestion::UseDesktopApp],
            ));
        }
        let has_audio =
            !probe.audio.is_empty() && !matches!(options.audio, AudioTrackChoice::Remove);
        let video_ok = |c: VideoCodec| match c {
            VideoCodec::H264 => web.h264_encode,
            VideoCodec::Vp9 => web.vp9_encode,
            VideoCodec::Av1 => false,
        };
        let audio_ok = |c: AudioCodec| {
            !has_audio
                || match c {
                    AudioCodec::Aac => web.aac_encode,
                    AudioCodec::Opus => web.opus_encode,
                }
        };
        if video_ok(target.video_codec) && audio_ok(target.audio_codec) {
            return Ok(target);
        }
        // No AAC encoder (Chrome on Linux) but Opus: keep the preferred video codec and
        // container. Opus in MP4 is valid ISO BMFF, and a preset that lists an Opus format
        // accepts Opus sound.
        if video_ok(target.video_codec)
            && target.container == Container::Mp4
            && has_audio
            && web.opus_encode
            && allowed.iter().any(|f| f.audio.eq_ignore_ascii_case("opus"))
        {
            return Ok(Target {
                audio_codec: AudioCodec::Opus,
                ..target
            });
        }
        // The preset's other formats, in its order (H.264 missing -> VP9 WebM, 3.5.6 step 5).
        for f in allowed {
            if let Some(t) = Target::from_format(f, &target.caps) {
                if video_ok(t.video_codec) && audio_ok(t.audio_codec) {
                    return Ok(t);
                }
            }
        }
        let missing = if video_ok(target.video_codec) {
            target.audio_codec.token()
        } else {
            target.video_codec.token()
        };
        Err((
            RefusalCode::BrowserLacksCodec {
                codec: missing.to_string(),
            },
            browser(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn transcode(
        &self,
        source: &SourceRef,
        _probe: &VideoProbe,
        _budget: Option<Budget>,
        _target: Target,
        _options: &PlanOptions,
        _allowed: &[cia_core::presets::VideoFormat],
        _faster: bool,
        _dest: &OutputDest,
        _progress: &dyn Fn(f32, Option<u64>),
        cancel: &CancelToken,
    ) -> Result<VideoTranscodeResult, EngineError> {
        let id = handle_of(source).ok_or_else(|| EngineError::Unsupported("path".into()))?;
        let r = VIDEO_RESULTS
            .with(|r| r.borrow_mut().remove(id))
            .ok_or_else(|| EngineError::Other("video result missing".into()))?;
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        match r.error.as_deref() {
            None => {}
            Some("cancelled") => return Err(EngineError::Cancelled),
            Some("damaged") => return Err(EngineError::Damaged(r.message.unwrap_or_default())),
            Some("refused") => {
                return Err(EngineError::Other(format!(
                    "refused:{}",
                    r.max_duration_ms.unwrap_or(0)
                )))
            }
            Some("over") => {
                return Err(EngineError::Other(format!(
                    "over:{}",
                    r.closest_bytes.unwrap_or(0)
                )))
            }
            Some(code) => {
                return Err(EngineError::Other(format!(
                    "{code}: {}",
                    r.message.unwrap_or_default()
                )))
            }
        }
        let bytes = VIDEO_BYTES.with(|b| b.borrow().get(id).map(|v| v.len() as u64));
        if r.kept_original {
            return Ok(VideoTranscodeResult {
                location: OutputLocation::Opfs {
                    path: format!("source:{id}"),
                },
                file_name: String::new(),
                bytes: bytes.unwrap_or(0),
                plan: r.plan,
                attempts: r.attempts,
                verification: VerificationReport {
                    size_ok: true,
                    decodes: true,
                    checks: vec!["original kept".into()],
                    failures: vec![],
                },
                kept_original: true,
            });
        }
        let bytes = bytes.ok_or_else(|| EngineError::Other("video output missing".into()))?;
        let plan = r.plan.ok_or_else(|| EngineError::Other("no plan".into()))?;
        Ok(VideoTranscodeResult {
            location: OutputLocation::Opfs {
                path: format!("{VIDEO_KEY}{id}"),
            },
            file_name: String::new(),
            bytes,
            plan: Some(plan),
            attempts: r.attempts,
            verification: r.verification.unwrap_or_default(),
            kept_original: false,
        })
    }

    fn encoder_kind(&self, _faster: bool) -> EncoderKind {
        EncoderKind::WebCodecs
    }

    fn capabilities(&self) -> Option<FfmpegCapabilities> {
        None
    }
}

/// Register what mediabunny found for the file under `id`: the `VideoProbe`
/// (null when the file could not be read) and whether `VideoDecoder` takes
/// its codec. Must happen before `inspect`, `preview` or `run` see the item.
#[wasm_bindgen]
pub fn set_video_probe(id: &str, probe: JsValue, can_decode: bool) -> Result<(), JsError> {
    let entry = if probe.is_null() || probe.is_undefined() {
        Err(())
    } else {
        Ok((from_js::<VideoProbe>(probe)?, can_decode))
    };
    PROBES.with(|p| p.borrow_mut().insert(id.to_string(), entry));
    Ok(())
}

#[wasm_bindgen]
pub fn has_video_probe(id: &str) -> bool {
    PROBES.with(|p| p.borrow().contains_key(id))
}

/// Register the finished transcode of the file under `id` for the next
/// `run_job`. `bytes` are the output (absent for kept originals and failures).
#[wasm_bindgen]
pub fn set_video_result(
    id: &str,
    result: JsValue,
    bytes: Option<js_sys::Uint8Array>,
) -> Result<(), JsError> {
    let r: VideoResultIn = from_js(result)?;
    VIDEO_RESULTS.with(|m| m.borrow_mut().insert(id.to_string(), r));
    VIDEO_BYTES.with(|b| {
        let mut b = b.borrow_mut();
        match bytes {
            Some(arr) => {
                b.insert(id.to_string(), arr.to_vec());
            }
            None => {
                b.remove(id);
            }
        }
    });
    Ok(())
}

#[wasm_bindgen]
pub fn clear_video_results() {
    VIDEO_RESULTS.with(|r| r.borrow_mut().clear());
    VIDEO_BYTES.with(|b| b.borrow_mut().clear());
}

/// The video items of a request with the budget, target and options the
/// engine would use (`cia_engine::VideoWork[]`), for the host to transcode
/// before `run_job`.
#[wasm_bindgen]
pub fn video_work(req: JsValue) -> Result<JsValue, JsError> {
    let req: cia_engine::PlanRequest = from_js(req)?;
    let work = engine().video_work(&req).map_err(js_err)?;
    to_js(&work)
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PlanOut {
    Plan { plan: VideoPlan },
    Refusal { refusal: PlanRefusal },
}

/// `cia_video_plan::plan` with the WebCodecs margin (0.92): `{type:"plan", plan}`
/// or `{type:"refusal", refusal}`.
#[wasm_bindgen]
pub fn plan_video(
    probe: JsValue,
    budget: JsValue,
    target: JsValue,
    options: JsValue,
) -> Result<JsValue, JsError> {
    let probe: VideoProbe = from_js(probe)?;
    let budget: Budget = from_js(budget)?;
    let target: Target = from_js(target)?;
    let options: PlanOptions = from_js(options)?;
    let out = match cia_video_plan::plan(&probe, &budget, &target, EncoderKind::WebCodecs, &options)
    {
        Ok(plan) => PlanOut::Plan { plan },
        Err(refusal) => PlanOut::Refusal { refusal },
    };
    to_js(&out)
}

/// Next attempt after an encode came out at `actual_bytes` (3.5.8, hardware
/// factor 0.93); null when no rung is left.
#[wasm_bindgen]
pub fn retry_video(plan: JsValue, actual_bytes: f64, budget: JsValue) -> Result<JsValue, JsError> {
    let plan: VideoPlan = from_js(plan)?;
    let budget: Budget = from_js(budget)?;
    let next =
        cia_video_plan::retry_scale(&plan, actual_bytes as u64, &budget, EncoderKind::WebCodecs);
    to_js(&next)
}

/// Keep, remux or encode (`cia_video_plan::keep_original_or_remux`).
#[wasm_bindgen]
pub fn decide_video(
    probe: JsValue,
    hard_bytes: f64,
    source_bytes: f64,
    allowed: JsValue,
) -> Result<JsValue, JsError> {
    let probe: VideoProbe = from_js(probe)?;
    let allowed: Vec<cia_core::presets::VideoFormat> = from_js(allowed)?;
    let d: Decision = cia_video_plan::keep_original_or_remux(
        &probe,
        hard_bytes as u64,
        source_bytes as u64,
        &allowed,
    );
    to_js(&d)
}

// ---------------------------------------------------------------------------
// Download helper

/// Build a zip (stored entries, the files are already compressed) from
/// parallel arrays of entry names and `Uint8Array` bodies. Used for
/// "Download all" when the result is several files (DESIGN.md 3.10).
#[wasm_bindgen]
pub fn package_zip(
    names: Vec<String>,
    bodies: js_sys::Array,
) -> Result<js_sys::Uint8Array, JsError> {
    if names.len() != bodies.length() as usize {
        return Err(JsError::new("names and bodies differ in length"));
    }
    let mut w = cia_archive::ArchiveWriter::in_memory(
        cia_archive::ArchiveKind::Zip,
        cia_archive::ArchiveOptions {
            mtime_unix: Some(now_ms() / 1000),
            ..Default::default()
        },
    )
    .map_err(js_err)?;
    for (i, name) in names.iter().enumerate() {
        let body = js_sys::Uint8Array::new(&bodies.get(i as u32)).to_vec();
        w.add_entry_stored(name, &body).map_err(js_err)?;
    }
    let bytes = w.finish().map_err(js_err)?;
    let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    arr.copy_from(&bytes);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_has_crate_version() {
        assert!(super::version().starts_with(env!("CARGO_PKG_VERSION")));
    }
}
