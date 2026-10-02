//! Audio ladders and the 15 percent rule (DESIGN.md 3.5.3 and 3.6).

use crate::plan::AudioCodec;
use cia_core::AudioTrackChoice;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One ladder rung: bitrate and channel count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioRung {
    pub bps: u64,
    pub channels: u32,
}

const fn rung(kbps: u64, channels: u32) -> AudioRung {
    AudioRung {
        bps: kbps * 1000,
        channels,
    }
}

/// AAC ladder for video (3.5.3): 128 stereo, 96 stereo, 64 stereo, 48 mono, 32 mono.
pub const AAC_VIDEO_LADDER: [AudioRung; 5] = [
    rung(128, 2),
    rung(96, 2),
    rung(64, 2),
    rung(48, 1),
    rung(32, 1),
];

/// Opus ladder for video (3.5.3): 96 stereo, 64 stereo, 48 stereo, 32 mono, 24 mono.
pub const OPUS_VIDEO_LADDER: [AudioRung; 5] = [
    rung(96, 2),
    rung(64, 2),
    rung(48, 2),
    rung(32, 1),
    rung(24, 1),
];

/// The 3.5.3 ladder for the audio codec of a video target, highest first.
pub fn video_audio_ladder(codec: AudioCodec) -> &'static [AudioRung] {
    match codec {
        AudioCodec::Aac => &AAC_VIDEO_LADDER,
        AudioCodec::Opus => &OPUS_VIDEO_LADDER,
    }
}

/// Lossy formats of the standalone audio planner (3.6 table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StandaloneAudioFormat {
    Mp3,
    Aac,
    Opus,
}

/// MP3 (3.6): 192, 160, 128, 112, 96, 80, 64 kb/s; mono from 80 down.
/// Sample rate is 44.1 kHz, 32 kHz at 64 kb/s.
pub const MP3_LADDER: [AudioRung; 7] = [
    rung(192, 2),
    rung(160, 2),
    rung(128, 2),
    rung(112, 2),
    rung(96, 2),
    rung(80, 1),
    rung(64, 1),
];

/// AAC .m4a (3.6): 160, 128, 96, 80, 64, 48 mono, 32 mono.
pub const AAC_LADDER: [AudioRung; 7] = [
    rung(160, 2),
    rung(128, 2),
    rung(96, 2),
    rung(80, 2),
    rung(64, 2),
    rung(48, 1),
    rung(32, 1),
];

/// Opus .ogg (3.6): 128, 96, 64, 48, 32 mono, 24 mono, 16 mono
/// (VOIP application below 32).
pub const OPUS_LADDER: [AudioRung; 7] = [
    rung(128, 2),
    rung(96, 2),
    rung(64, 2),
    rung(48, 2),
    rung(32, 1),
    rung(24, 1),
    rung(16, 1),
];

/// The 3.6 ladder for a standalone audio format, highest first. The floor is
/// the last rung.
pub fn standalone_ladder(format: StandaloneAudioFormat) -> &'static [AudioRung] {
    match format {
        StandaloneAudioFormat::Mp3 => &MP3_LADDER,
        StandaloneAudioFormat::Aac => &AAC_LADDER,
        StandaloneAudioFormat::Opus => &OPUS_LADDER,
    }
}

/// What the audio stream of the output gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioChoice {
    /// 0 when there is no audio in the output.
    pub bps: u64,
    /// 0 when there is no audio in the output.
    pub channels: u32,
}

impl AudioChoice {
    pub const NONE: AudioChoice = AudioChoice {
        bps: 0,
        channels: 0,
    };
}

/// Share of `total_bps` the audio may take.
pub const AUDIO_SHARE_PERCENT: u64 = 15;

/// Pick the highest rung whose bitrate is at most 15 percent of `total_bps`;
/// the lowest rung if none fits. `source_channels` is the channel count of
/// the source track (or the widest track when mixing); mono never upmixes.
/// `Remove`, or a source with no audio (`source_channels == 0`), gives
/// [`AudioChoice::NONE`].
pub fn pick_audio(
    total_bps: u64,
    codec: AudioCodec,
    source_channels: u32,
    choice: &AudioTrackChoice,
) -> AudioChoice {
    if matches!(choice, AudioTrackChoice::Remove) || source_channels == 0 {
        return AudioChoice::NONE;
    }
    let ladder = video_audio_ladder(codec);
    let cap = total_bps * AUDIO_SHARE_PERCENT / 100;
    let rung = ladder
        .iter()
        .find(|r| r.bps <= cap)
        .unwrap_or_else(|| ladder.last().expect("ladders are not empty"));
    AudioChoice {
        bps: rung.bps,
        channels: rung.channels.min(source_channels),
    }
}

/// Smallest `total_bps` at which `pick_audio` leaves at least `min_video_bps`
/// for the video stream. Used by the refusal math (3.5.5): because the audio
/// rung grows with the total, "lowest rung plus video floor" is not always
/// reachable, so the bound is solved against the rule that actually picks.
pub(crate) fn min_total_for_video(min_video_bps: u64, codec: AudioCodec, has_audio: bool) -> u64 {
    if !has_audio {
        return min_video_bps;
    }
    let ladder = video_audio_ladder(codec);
    // Rung i (counting from the lowest) is picked when threshold(i) <= total < threshold(i + 1),
    // where threshold is the smallest total whose 15 percent reaches the rung.
    // pick_audio takes rung r when r.bps <= floor(total * 15 / 100), i.e. total >= ceil(r.bps * 100 / 15).
    let threshold = |r: &AudioRung| (r.bps * 100).div_ceil(AUDIO_SHARE_PERCENT);
    let mut best: Option<u64> = None;
    let n = ladder.len();
    for i in 0..n {
        let r = ladder[n - 1 - i]; // lowest first
        let lower = if i == 0 {
            min_video_bps + r.bps
        } else {
            threshold(&r).max(min_video_bps + r.bps)
        };
        let upper = if i + 1 < n {
            Some(threshold(&ladder[n - 2 - i]))
        } else {
            None
        };
        if upper.is_none_or(|u| lower < u) {
            best = Some(best.map_or(lower, |b| b.min(lower)));
        }
    }
    best.unwrap_or(min_video_bps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_total_leaves_room_for_video() {
        // 640x360x30 at the h264 floor 0.070 = 483,840 bps.
        let t = min_total_for_video(483_840, AudioCodec::Aac, true);
        assert_eq!(t, 547_840);
        let a = pick_audio(t, AudioCodec::Aac, 2, &AudioTrackChoice::MixAll);
        assert_eq!(a.bps, 64_000);
        assert!(t - a.bps >= 483_840);
        // One less and the video no longer fits.
        let a = pick_audio(t - 1, AudioCodec::Aac, 2, &AudioTrackChoice::MixAll);
        assert!(t - 1 - a.bps < 483_840);
        assert_eq!(min_total_for_video(100, AudioCodec::Aac, false), 100);
    }
}
