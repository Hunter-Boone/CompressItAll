//! What the prober learned about the input (DESIGN.md 3.5.1). Filled by
//! ffprobe on desktop and mediabunny on the web; this crate only reads it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One audio stream of the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct AudioTrack {
    /// Stream index among the audio streams (0-based), the `n` in `0:a:n`.
    pub index: u32,
    /// Lower-case codec token as ffprobe names it: "aac", "opus", "vorbis", "pcm_s16le"...
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate_bps: Option<u64>,
    /// `title` tag, when the file has one (OBS tags "Game" / "Mic").
    pub title: Option<String>,
}

/// Everything the planner needs to know about the input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct VideoProbe {
    pub duration_ms: u64,
    /// Lower-case container token: "mp4", "mov", "mkv", "webm", "avi"...
    pub container: String,
    /// Lower-case codec token: "h264", "hevc", "vp9", "av1", "prores"...
    pub video_codec: String,
    pub coded_w: u32,
    pub coded_h: u32,
    /// Display size after the rotation in the display matrix has been applied.
    pub display_w: u32,
    pub display_h: u32,
    pub rotation_degrees: i32,
    pub avg_fps: f32,
    pub max_fps: f32,
    /// More than 2 percent spread in packet durations over the first 5 s.
    pub is_vfr: bool,
    pub pixel_format: String,
    pub color_transfer: String,
    pub color_primaries: String,
    /// Transfer `smpte2084` (PQ) or `arib-std-b67` (HLG).
    pub is_hdr: bool,
    pub dolby_vision_profile: Option<u8>,
    pub audio: Vec<AudioTrack>,
}

impl VideoProbe {
    /// Short edge of the display size ("height" in the ladder, DESIGN 3.5.4).
    pub fn short_edge(&self) -> u32 {
        self.display_w.min(self.display_h)
    }

    pub fn is_portrait(&self) -> bool {
        self.display_h > self.display_w
    }

    pub fn duration_s(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
}
