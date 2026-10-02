#![allow(clippy::chunks_exact_to_as_chunks)]
//! Smidge image planner (DESIGN.md 3.4): decode and normalise, classify,
//! try candidate encoders in order, binary-search quality in log-size space,
//! downscale only as a last resort, pick by SSIMULACRA2, verify with a second
//! decoder. Shared by the native engine and the WASM engine.

pub mod animated;
pub mod candidates;
pub mod classify;
pub mod decode;
pub mod encode;
pub mod metadata;
pub mod resize;
pub mod score;
pub mod search;
pub mod verify;

pub use candidates::{Candidate, OutputImageFormat};
pub use classify::{classify, Class, Classification};
pub use decode::{decode, inspect, DecodedImage, ImageInfo, SourceFormat};
pub use search::{
    compress, floor_size, lossless_size, ImageAttempt, ImageOptions, ImageOutcome, ImageResult,
    Mode,
};
pub use verify::{verify, Expect};

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("damaged_input: {0}")]
    Damaged(String),
    #[error("unsupported_input: {0}")]
    Unsupported(String),
    #[error("encoder: {0}")]
    Encoder(String),
    #[error("cancelled")]
    Cancelled,
}

/// Cancellation is polled between encodes.
pub type CancelFn<'a> = &'a (dyn Fn() -> bool + Sync);
/// Progress in [0, 1] with a short label.
pub type ProgressFn<'a> = &'a (dyn Fn(f32, &str) + Sync);
