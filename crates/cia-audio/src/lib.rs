//! Smidge audio engine (DESIGN.md section 3.6).
//!
//! Decode with Symphonia (MP3, FLAC, Ogg Vorbis, WAV, AIFF, CAF, MKV audio)
//! and libopus (Ogg Opus, native only), encode lossless FLAC and WAV, encode
//! Opus into Ogg natively, resample with rubato, and provide the planner math
//! and the verification predicate the hosts use before reporting `Fitted`.
//!
//! `aac`, `alac` and `isomp4` are deliberately not compiled in; AAC/M4A/ALAC
//! and WMA input is reported with `needs_ffmpeg = true` so the desktop host
//! can route it through FFmpeg and the web host through WebCodecs.

#![forbid(unsafe_code)]

mod decode;
mod encode;
#[cfg(feature = "opus-native")]
mod opus_codec;
pub mod plan;
mod resample;
mod verify;

pub use decode::{decode, probe, AudioInfo};
pub use encode::{encode_flac, encode_wav};
#[cfg(feature = "opus-native")]
pub use opus_codec::{decode_opus_ogg, encode_opus_ogg, probe_opus_ogg, OpusApplication};
pub use resample::{downmix_mono, resample, with_channels};
pub use verify::{verify, VerifyReport};

/// Decoded PCM, interleaved 32-bit float in the range -1..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples; `samples.len()` is a multiple of `channels`.
    pub samples: Vec<f32>,
    pub duration_ms: u64,
    /// Lower-case codec id of the input: "mp3", "flac", "vorbis", "opus", "pcm".
    pub source_codec: String,
}

impl Decoded {
    /// Build from interleaved samples; computes `duration_ms`.
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<f32>, source_codec: &str) -> Self {
        let frames = if channels == 0 {
            0
        } else {
            samples.len() / channels as usize
        };
        Decoded {
            sample_rate,
            channels,
            samples,
            duration_ms: frames_to_ms(frames as u64, sample_rate),
            source_codec: source_codec.to_string(),
        }
    }

    /// Number of sample frames (samples per channel).
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }

    /// The first `seconds` of the audio (used for the lossless size estimate).
    pub fn head(&self, seconds: f32) -> Decoded {
        let keep = ((seconds.max(0.0) as f64 * self.sample_rate as f64).round() as usize)
            .min(self.frames());
        Decoded::new(
            self.sample_rate,
            self.channels,
            self.samples[..keep * self.channels as usize].to_vec(),
            &self.source_codec,
        )
    }
}

/// Frames to milliseconds, rounded to nearest.
pub fn frames_to_ms(frames: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    (frames as u128 * 1000 + sample_rate as u128 / 2) as u64 / sample_rate as u64
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("unsupported audio input: {0}")]
    Unsupported(String),
    /// AAC, ALAC and WMA: the host decodes these with FFmpeg or WebCodecs.
    #[error("{0} input needs FFmpeg or WebCodecs")]
    NeedsFfmpeg(String),
    #[error("could not decode audio: {0}")]
    Decode(String),
    #[error("could not encode audio: {0}")]
    Encode(String),
    #[error("the input has no audio samples")]
    Empty,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}
