#![forbid(unsafe_code)]
//! Smidge video planner (DESIGN.md 3.5). Pure math only: the native FFmpeg
//! runner (`cia-ffmpeg`) and the browser WebCodecs pipeline
//! (`packages/webvideo`) both call into this crate so the prediction shown on
//! step 3 and the encode that follows come from one place. Nothing here does
//! I/O or talks to an encoder, and it builds for `wasm32-unknown-unknown`.
//!
//! Flow: [`VideoProbe`] + [`Budget`] + [`Target`] → [`plan`] → [`VideoPlan`] or
//! [`PlanRefusal`]; after an encode that overshoots, [`retry_scale`] gives the
//! next attempt; [`keep_original_or_remux`] decides whether an encode is
//! needed at all; [`encoder_chain`] orders the native H.264 encoders to try.

pub mod audio;
pub mod budget;
pub mod decision;
pub mod encoders;
pub mod ladder;
pub mod plan;
pub mod probe;
pub mod retry;

pub use audio::{
    pick_audio, standalone_ladder, video_audio_ladder, AudioChoice, AudioRung,
    StandaloneAudioFormat,
};
pub use budget::{container_overhead, total_bps, Budget, Margin};
pub use decision::{keep_original_or_remux, Decision};
pub use encoders::{encoder_chain, Gpu, Platform};
pub use ladder::{build_rungs, floor_bpp, quality_label, Rung};
pub use plan::{
    plan, predict_for_preset, suggest_presets, AudioCodec, Container, EncoderKind, PlanOptions,
    PlanRefusal, Target, VideoCodec, VideoPlan,
};
pub use probe::{AudioTrack, VideoProbe};
pub use retry::{retry_scale, UNDERSHOOT_ACCEPT_FRACTION};
