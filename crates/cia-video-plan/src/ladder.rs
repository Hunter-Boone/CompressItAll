//! Resolution and frame-rate ladder, bits-per-pixel floors and the quality
//! label (DESIGN.md 3.5.4).

use crate::plan::VideoCodec;
use crate::probe::VideoProbe;
use cia_core::presets::VideoCaps;
use cia_core::{FrameRatePref, QualityLabel};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One output size and frame rate the planner may choose.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Rung {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

impl Rung {
    /// "Height" in the ladder sense: the short edge, so portrait video steps
    /// 1080, 720, 540... on its narrow side.
    pub fn short_edge(&self) -> u32 {
        self.width.min(self.height)
    }

    pub fn pixels_per_second(&self) -> f64 {
        f64::from(self.width) * f64::from(self.height) * f64::from(self.fps)
    }
}

/// Short edges the ladder steps through below the source size.
pub const LADDER_HEIGHTS: [u32; 5] = [1080, 720, 540, 480, 360];

/// Nothing above this is ever produced.
pub const MAX_FPS: f32 = 60.0;

/// Minimum bits per pixel per frame at which a rung is still watchable.
///
/// ```text
/// h264: h >= 720 -> 0.050, h >= 480 -> 0.060, else 0.070
/// vp9:  0.75 * h264;   av1: 0.60 * h264
/// ```
pub fn floor_bpp(codec: VideoCodec, short_edge: u32) -> f64 {
    let h264 = if short_edge >= 720 {
        0.050
    } else if short_edge >= 480 {
        0.060
    } else {
        0.070
    };
    match codec {
        VideoCodec::H264 => h264,
        VideoCodec::Vp9 => 0.75 * h264,
        VideoCodec::Av1 => 0.60 * h264,
    }
}

/// `bpp >= 2.2 * floor` is Great, `>= 1.4 * floor` Good, else Okay.
pub fn quality_label(bpp: f64, floor: f64) -> QualityLabel {
    if bpp >= 2.2 * floor {
        QualityLabel::Great
    } else if bpp >= 1.4 * floor {
        QualityLabel::Good
    } else {
        QualityLabel::Okay
    }
}

/// Round to even the way FFmpeg's `scale=-2:H` does
/// (`av_rescale(h, in_w, in_h * 2) * 2`): the nearest multiple of two.
fn even_up(v: f64) -> u32 {
    (((v / 2.0).round() as u32) * 2).max(2)
}

/// Even number at or below `v`, at least 2. Used for the source size itself,
/// where rounding up would be a (one pixel) upscale.
fn even_down(v: u32) -> u32 {
    (v / 2 * 2).max(2)
}

/// The source scaled so its short edge is `short`, aspect kept, even sizes.
fn scaled_to(probe: &VideoProbe, short: u32, fps: f32) -> Rung {
    let (w, h) = (f64::from(probe.display_w), f64::from(probe.display_h));
    if probe.is_portrait() {
        Rung {
            width: even_down(short),
            height: even_up(h * f64::from(short) / w),
            fps,
        }
    } else {
        Rung {
            width: even_up(w * f64::from(short) / h),
            height: even_down(short),
            fps,
        }
    }
}

/// Build the ladder, highest quality first.
///
/// ```text
/// start = (display_w, display_h, min(src_fps, caps.max_fps, 60))
/// rungs.push(start)
/// if start.fps > 30: rungs.push(same size, fps 30)         // 60 -> 30 first
/// for h in [1080, 720, 540, 480, 360] where h < short edge:  // never upscale
///     rungs.push(scaled to short edge h, fps min(start.fps, 30))
/// ```
///
/// `caps.max_short_edge` (WhatsApp 720) caps the start rung: a source above
/// it is scaled down to the cap at the start frame rate, so a 1440p60 clip
/// still has a 1080p60 rung for a Discord preset (see docs/DECISIONS.md).
/// `KeepOriginal` skips the 30 fps rung and keeps the start frame rate all
/// the way down; `Fps30` starts at 30. Dimensions are rounded to even
/// numbers, which yuv420p needs; the source size itself rounds down so no
/// rung is ever larger than the source.
pub fn build_rungs(
    probe: &VideoProbe,
    caps: &VideoCaps,
    frame_rate_pref: &FrameRatePref,
) -> Vec<Rung> {
    let mut rungs = Vec::new();
    if probe.display_w == 0 || probe.display_h == 0 {
        return rungs;
    }
    let src_fps = if probe.avg_fps > 0.0 {
        probe.avg_fps
    } else {
        30.0
    };
    let mut start_fps = src_fps.min(MAX_FPS);
    if let Some(max_fps) = caps.max_fps {
        start_fps = start_fps.min(max_fps as f32);
    }
    if matches!(frame_rate_pref, FrameRatePref::Fps30) {
        start_fps = start_fps.min(30.0);
    }
    let keep_fps = matches!(frame_rate_pref, FrameRatePref::KeepOriginal);
    let lower_fps = if keep_fps {
        start_fps
    } else {
        start_fps.min(30.0)
    };

    let source_short = probe.short_edge();
    let start_short = caps
        .max_short_edge
        .map_or(source_short, |cap| cap.min(source_short));
    let start = if start_short == source_short {
        Rung {
            width: even_down(probe.display_w),
            height: even_down(probe.display_h),
            fps: start_fps,
        }
    } else {
        scaled_to(probe, start_short, start_fps)
    };
    rungs.push(start);
    if !keep_fps && start_fps > 30.0 {
        rungs.push(Rung { fps: 30.0, ..start });
    }
    for h in LADDER_HEIGHTS {
        if h < start.short_edge() {
            rungs.push(scaled_to(probe, h, lower_fps));
        }
    }
    rungs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        assert!(close(floor_bpp(VideoCodec::H264, 1440), 0.050));
        assert!(close(floor_bpp(VideoCodec::H264, 720), 0.050));
        assert!(close(floor_bpp(VideoCodec::H264, 540), 0.060));
        assert!(close(floor_bpp(VideoCodec::H264, 480), 0.060));
        assert!(close(floor_bpp(VideoCodec::H264, 360), 0.070));
        assert!(close(floor_bpp(VideoCodec::Vp9, 720), 0.0375));
        assert!(close(floor_bpp(VideoCodec::Av1, 720), 0.030));
    }

    #[test]
    fn labels() {
        // 2.2 * 0.05 and 1.4 * 0.05 are not exact in binary; test clearly above and below.
        assert_eq!(quality_label(0.1101, 0.05), QualityLabel::Great);
        assert_eq!(quality_label(0.1099, 0.05), QualityLabel::Good);
        assert_eq!(quality_label(0.0746, 0.05), QualityLabel::Good);
        assert_eq!(quality_label(0.0699, 0.05), QualityLabel::Okay);
    }

    #[test]
    fn even_rounding() {
        assert_eq!(even_up(853.33), 854);
        assert_eq!(even_up(1280.0), 1280);
        assert_eq!(even_up(1706.67), 1706); // round(853.33) * 2
        assert_eq!(even_up(640.5), 640); // 854x480 -> 360: round(320.25) * 2
        assert_eq!(even_down(1081), 1080);
    }
}
