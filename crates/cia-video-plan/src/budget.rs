//! Byte budget → bits per second (DESIGN.md 3.5.2).

use crate::plan::{Container, EncoderKind};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The byte numbers a resolved preset gives the planner, named as in
/// `cia_core::ResolvedLimit` (`hard_for` / `budget_for` / `safety_for` for the
/// video kind).
///
/// The planner aims at `raw_budget_bytes`: the preset limit with
/// `safety_bytes` already taken off. DESIGN 3.5 calls this number `hard_bytes`
/// and subtracts one more byte; see docs/DECISIONS.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Budget {
    /// What the planner aims at; the output must be at most this.
    pub raw_budget_bytes: u64,
    /// The service's stated limit for one file.
    pub hard_bytes: u64,
    pub safety_bytes: u64,
}

impl Budget {
    pub fn from_resolved(limit: &cia_core::ResolvedLimit) -> Self {
        let kind = cia_core::Kind::Video;
        Budget {
            raw_budget_bytes: limit.budget_for(kind),
            hard_bytes: limit.hard_for(kind),
            safety_bytes: limit.safety_for(kind),
        }
    }
}

/// Fraction of the budget bits the encoder is asked to use, leaving room for
/// rate-control error: 0.96 for two-pass software encoders, 0.92 for
/// single-pass hardware encoders and WebCodecs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margin(f64);

impl Margin {
    pub const TWO_PASS_SOFTWARE: Margin = Margin(0.96);
    pub const SINGLE_PASS: Margin = Margin(0.92);

    pub fn for_encoder(kind: EncoderKind) -> Margin {
        match kind {
            EncoderKind::TwoPassSoftware => Margin::TWO_PASS_SOFTWARE,
            EncoderKind::SinglePassHardware | EncoderKind::WebCodecs => Margin::SINGLE_PASS,
        }
    }

    pub fn value(self) -> f64 {
        self.0
    }
}

/// Bytes the container itself costs, outside the elementary streams.
///
/// ```text
/// mp4:  4096 + ceil(duration_s) * (fps * 14 + audio_tracks_out * 47 * 10)
/// webm: 4096 + ceil(duration_s) * (fps * 12 + audio_tracks_out * 50 * 8)
/// ```
///
/// Fractional frame rates round the per-second video term up (29.97 fps costs
/// the same as 30).
pub fn container_overhead(
    container: Container,
    duration_s: f64,
    fps: f32,
    audio_tracks_out: u32,
) -> u64 {
    let seconds = duration_s.max(0.0).ceil() as u64;
    let fps = f64::from(fps.max(0.0));
    let (video_per_frame, audio_per_second) = match container {
        Container::Mp4 => (14.0, 47 * 10),
        Container::Webm => (12.0, 50 * 8),
    };
    let video_per_second = (fps * video_per_frame).ceil() as u64;
    4096 + seconds * (video_per_second + u64::from(audio_tracks_out) * audio_per_second)
}

/// `floor((raw_budget - overhead) * 8 * margin / duration_s)`; 0 when the
/// overhead alone exceeds the budget or the duration is not positive.
pub fn total_bps(
    raw_budget_bytes: u64,
    overhead_bytes: u64,
    margin: Margin,
    duration_s: f64,
) -> u64 {
    if duration_s <= 0.0 {
        return 0;
    }
    let usable = raw_budget_bytes.saturating_sub(overhead_bytes) as f64;
    (usable * 8.0 * margin.value() / duration_s).floor() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overhead_matches_design_example() {
        // DESIGN 3.5.4: 70 s, 30 fps, one audio track out.
        assert_eq!(container_overhead(Container::Mp4, 70.0, 30.0, 1), 66_396);
        // 60 fps start rung.
        assert_eq!(container_overhead(Container::Mp4, 70.0, 60.0, 1), 95_796);
        // No audio.
        assert_eq!(container_overhead(Container::Mp4, 70.0, 30.0, 0), 33_496);
        // WebM: 4096 + 70 * (360 + 400).
        assert_eq!(container_overhead(Container::Webm, 70.0, 30.0, 1), 57_296);
        // Fractional seconds round up; 29.97 fps costs the same as 30.
        assert_eq!(container_overhead(Container::Mp4, 69.2, 29.97, 1), 66_396);
    }

    #[test]
    fn margins() {
        assert_eq!(
            Margin::for_encoder(EncoderKind::TwoPassSoftware).value(),
            0.96
        );
        assert_eq!(
            Margin::for_encoder(EncoderKind::SinglePassHardware).value(),
            0.92
        );
        assert_eq!(Margin::for_encoder(EncoderKind::WebCodecs).value(), 0.92);
    }

    #[test]
    fn total_bps_floors_and_saturates() {
        assert_eq!(
            total_bps(20_905_984, 66_396, Margin::SINGLE_PASS, 70.0),
            2_191_133
        );
        assert_eq!(total_bps(1000, 2000, Margin::SINGLE_PASS, 70.0), 0);
        assert_eq!(total_bps(1000, 0, Margin::SINGLE_PASS, 0.0), 0);
    }
}
