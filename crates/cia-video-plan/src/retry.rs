//! Retry after an overshoot (DESIGN.md 3.5.8).

use crate::budget::{container_overhead, Budget};
use crate::ladder::{floor_bpp, quality_label};
use crate::plan::{EncoderKind, VideoPlan};

/// An output below this fraction of the budget is an undershoot. It is
/// accepted as is: re-encoding to fill the budget doubles the wait for a gain
/// few people can see. Callers never call [`retry_scale`] for it.
pub const UNDERSHOOT_ACCEPT_FRACTION: f64 = 0.85;

/// Next attempt after an encode came out at `actual_bytes`, over the budget.
///
/// ```text
/// factor = (raw_budget / actual) * (0.97 software | 0.93 hardware)
/// video_bps = video_bps * factor
/// if video_bps < floor at current rung: move down one rung, recompute
/// ```
///
/// Steps down the ladder until a rung's floor is met; None when no rung
/// remains. Audio keeps its rung. The factor never exceeds 1, so an output
/// that was already under budget is never scaled up (an undershoot is
/// accepted as is, see [`UNDERSHOOT_ACCEPT_FRACTION`]).
pub fn retry_scale(
    plan: &VideoPlan,
    actual_bytes: u64,
    budget: &Budget,
    encoder: EncoderKind,
) -> Option<VideoPlan> {
    if actual_bytes == 0 {
        return None;
    }
    let scale = match encoder {
        EncoderKind::TwoPassSoftware => 0.97,
        EncoderKind::SinglePassHardware | EncoderKind::WebCodecs => 0.93,
    };
    let factor = (budget.raw_budget_bytes as f64 / actual_bytes as f64 * scale).min(1.0);
    let video_bps = (plan.video_bps as f64 * factor).floor() as u64;
    let duration_s = plan.duration_ms as f64 / 1000.0;
    let tracks_out = u32::from(plan.has_audio());
    let mut idx = plan.rung_index;
    while let Some(rung) = plan.ladder.get(idx) {
        let floor = floor_bpp(plan.video_codec, rung.short_edge());
        let pps = rung.pixels_per_second();
        let bpp = video_bps as f64 / pps;
        if bpp >= floor {
            let overhead = container_overhead(plan.container, duration_s, rung.fps, tracks_out);
            let bits = (video_bps + plan.audio_bps) as u128 * plan.duration_ms as u128;
            return Some(VideoPlan {
                width: rung.width,
                height: rung.height,
                fps: rung.fps,
                video_bps,
                quality: quality_label(bpp, floor),
                predicted_bytes: bits.div_ceil(8000) as u64 + overhead,
                rung_index: idx,
                overhead_bytes: overhead,
                bpp,
                floor_bpp: floor,
                ..plan.clone()
            });
        }
        idx += 1;
    }
    None
}
