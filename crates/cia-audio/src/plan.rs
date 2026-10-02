//! Audio planner math (DESIGN.md section 3.6): bitrate ladders, the fit
//! algorithm, refusal with a maximum duration that itself fits, retry scaling
//! and the Smaller-mode decision. Pure functions, no codecs, table-tested.

use cia_core::{RefusalCode, SmallerLevel};

use crate::AudioInfo;

/// Output formats, in the ids used by `presets.json` `formats.audio`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    Mp3,
    M4aAac,
    OggOpus,
    Flac,
    Wav,
}

impl AudioFormat {
    pub fn from_id(id: &str) -> Option<AudioFormat> {
        Some(match id {
            "mp3" => AudioFormat::Mp3,
            "m4a_aac" | "aac" | "m4a" => AudioFormat::M4aAac,
            "ogg_opus" | "opus" | "ogg" => AudioFormat::OggOpus,
            "flac" => AudioFormat::Flac,
            "wav" => AudioFormat::Wav,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4aAac => "m4a_aac",
            AudioFormat::OggOpus => "ogg_opus",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4aAac => "m4a",
            AudioFormat::OggOpus => "ogg",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
        }
    }

    pub fn is_lossless(self) -> bool {
        matches!(self, AudioFormat::Flac | AudioFormat::Wav)
    }

    /// Bitrate ladder, highest first: (bits per second, channels, sample rate).
    /// Empty for lossless formats.
    pub fn ladder(self) -> &'static [Rung] {
        match self {
            AudioFormat::Mp3 => &MP3_LADDER,
            AudioFormat::M4aAac => &AAC_LADDER,
            AudioFormat::OggOpus => &OPUS_LADDER,
            AudioFormat::Flac | AudioFormat::Wav => &[],
        }
    }

    /// Lowest rung of the ladder; `None` for lossless formats.
    pub fn floor(self) -> Option<Rung> {
        self.ladder().last().copied()
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rung {
    pub bitrate_bps: u32,
    pub channels: u16,
    pub sample_rate: u32,
}

const fn rung(kbps: u32, channels: u16, sample_rate: u32) -> Rung {
    Rung {
        bitrate_bps: kbps * 1000,
        channels,
        sample_rate,
    }
}

/// MP3 (FFmpeg libmp3lame CBR): mono from 80 down, 32 kHz at 64.
pub const MP3_LADDER: [Rung; 7] = [
    rung(192, 2, 44_100),
    rung(160, 2, 44_100),
    rung(128, 2, 44_100),
    rung(112, 2, 44_100),
    rung(96, 2, 44_100),
    rung(80, 1, 44_100),
    rung(64, 1, 32_000),
];

/// AAC (FFmpeg `aac` / WebCodecs): 48 and 32 mono.
pub const AAC_LADDER: [Rung; 7] = [
    rung(160, 2, 44_100),
    rung(128, 2, 44_100),
    rung(96, 2, 44_100),
    rung(80, 2, 44_100),
    rung(64, 2, 44_100),
    rung(48, 1, 44_100),
    rung(32, 1, 44_100),
];

/// Opus (libopus / WebCodecs), always 48 kHz: 32 and below mono, VOIP below 32.
pub const OPUS_LADDER: [Rung; 7] = [
    rung(128, 2, 48_000),
    rung(96, 2, 48_000),
    rung(64, 2, 48_000),
    rung(48, 2, 48_000),
    rung(32, 1, 48_000),
    rung(24, 1, 48_000),
    rung(16, 1, 48_000),
];

/// Opus bitrates below this use the VOIP application.
pub const OPUS_VOIP_BELOW_BPS: u32 = 32_000;

/// Bits used per budget bit: 2 percent headroom on top of the container overhead.
pub const FILL: f64 = 0.98;
/// Retry shrink factor on top of budget/actual.
pub const RETRY_MARGIN: f64 = 0.97;
pub const MAX_ATTEMPTS: u8 = 4;
/// Re-encoded FLAC is kept only if it is at least this much smaller.
pub const FLAC_REENCODE_MIN_SAVING: f64 = 0.03;

/// What this host can encode (feature matrix 2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostAudioCaps {
    pub mp3: bool,
    pub aac: bool,
    pub opus: bool,
    pub flac: bool,
    pub wav: bool,
}

impl HostAudioCaps {
    pub fn can(&self, f: AudioFormat) -> bool {
        match f {
            AudioFormat::Mp3 => self.mp3,
            AudioFormat::M4aAac => self.aac,
            AudioFormat::OggOpus => self.opus,
            AudioFormat::Flac => self.flac,
            AudioFormat::Wav => self.wav,
        }
    }

    /// Desktop without FFmpeg, and the web: Opus, FLAC, WAV.
    pub fn native_only() -> Self {
        HostAudioCaps {
            mp3: false,
            aac: false,
            opus: true,
            flac: true,
            wav: true,
        }
    }

    /// Desktop with a working FFmpeg: everything.
    pub fn all() -> Self {
        HostAudioCaps {
            mp3: true,
            aac: true,
            opus: true,
            flac: true,
            wav: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPlan {
    pub format: AudioFormat,
    /// Target bitrate. For lossless formats this is the estimated average.
    pub bitrate_bps: u32,
    pub channels: u16,
    pub sample_rate: u32,
    pub predicted_bytes: u64,
    /// 1 for the first encode; `retry_scale` increments it.
    pub attempt: u8,
}

impl AudioPlan {
    /// Opus application for this plan's bitrate (VOIP below 32 kb/s).
    pub fn opus_voip(&self) -> bool {
        self.format == AudioFormat::OggOpus && self.bitrate_bps < OPUS_VOIP_BELOW_BPS
    }
}

/// Container bytes that are not audio payload.
/// Ogg: about 1 percent of the file plus 1 KB; MP3: an ID3 tag of about 200
/// bytes; M4A: about 8 KB of moov plus 40 bytes per second of sample tables;
/// FLAC: STREAMINFO and padding; WAV: the 44-byte header.
pub fn container_overhead(format: AudioFormat, duration_ms: u64, file_bytes: u64) -> u64 {
    let secs = duration_ms as f64 / 1000.0;
    match format {
        AudioFormat::OggOpus => (file_bytes as f64 * 0.01).ceil() as u64 + 1024,
        AudioFormat::Mp3 => 200,
        AudioFormat::M4aAac => 8192 + (40.0 * secs).ceil() as u64,
        AudioFormat::Flac => 1024,
        AudioFormat::Wav => 44,
    }
}

/// Bytes an encode at `bitrate_bps` for `duration_ms` is expected to produce,
/// container included.
pub fn predict_bytes(format: AudioFormat, bitrate_bps: u32, duration_ms: u64) -> u64 {
    let payload = (bitrate_bps as f64 * duration_ms as f64 / 8000.0).ceil() as u64;
    let overhead = match format {
        // Ogg overhead scales with the payload, not the budget.
        AudioFormat::OggOpus => (payload as f64 * 0.01).ceil() as u64 + 1024,
        other => container_overhead(other, duration_ms, payload),
    };
    payload + overhead
}

/// Bits available for audio payload in `raw_budget_bytes` for one format.
fn budget_bits(format: AudioFormat, duration_ms: u64, raw_budget_bytes: u64) -> f64 {
    let overhead = container_overhead(format, duration_ms, raw_budget_bytes);
    (raw_budget_bytes.saturating_sub(overhead) as f64) * 8.0 * FILL
}

fn formats_in_order(allowed_formats: &[&str], host_can: &HostAudioCaps) -> Vec<AudioFormat> {
    allowed_formats
        .iter()
        .filter_map(|id| AudioFormat::from_id(id))
        .filter(|f| host_can.can(*f))
        .collect()
}

/// Pick the first allowed format this host can produce whose highest fitting
/// rung exists, or a lossless format whose estimate fits. Refuses with the
/// longest duration that would fit at the lowest floor among the usable
/// lossy formats.
///
/// `raw_budget_bytes` is `hard_bytes` from `cia-core::limits` (safety already
/// taken off). `lossless_estimate` is the predicted whole-file size of a FLAC
/// encode (encode the first 10 s with `encode_flac` and extrapolate).
pub fn plan_fit(
    duration_ms: u64,
    raw_budget_bytes: u64,
    allowed_formats: &[&str],
    host_can: &HostAudioCaps,
    source_channels: u16,
    lossless_estimate: Option<u64>,
) -> Result<AudioPlan, RefusalCode> {
    let formats = formats_in_order(allowed_formats, host_can);
    let duration_s = duration_ms.max(1) as f64 / 1000.0;
    let source_channels = source_channels.max(1);

    for format in &formats {
        if format.is_lossless() {
            let estimate = match (*format, lossless_estimate) {
                (AudioFormat::Flac, Some(e)) => e,
                (AudioFormat::Wav, Some(e)) => e,
                _ => continue,
            };
            if estimate <= raw_budget_bytes {
                let bps = (estimate as f64 * 8.0 / duration_s).round() as u32;
                return Ok(AudioPlan {
                    format: *format,
                    bitrate_bps: bps,
                    channels: source_channels,
                    sample_rate: 0,
                    predicted_bytes: estimate,
                    attempt: 1,
                });
            }
            continue;
        }
        let target_bps = budget_bits(*format, duration_ms, raw_budget_bytes) / duration_s;
        if let Some(r) = format
            .ladder()
            .iter()
            .find(|r| r.bitrate_bps as f64 <= target_bps)
        {
            let channels = r.channels.min(source_channels);
            return Ok(AudioPlan {
                format: *format,
                bitrate_bps: r.bitrate_bps,
                channels,
                sample_rate: r.sample_rate,
                predicted_bytes: predict_bytes(*format, r.bitrate_bps, duration_ms),
                attempt: 1,
            });
        }
    }

    Err(RefusalCode::TooLongForLimit {
        max_duration_ms: max_duration_ms(raw_budget_bytes, &formats),
    })
}

/// The longest duration whose floor-rate encode fits `raw_budget_bytes` in the
/// cheapest usable lossy format. Guaranteed to plan successfully when fed back
/// into `plan_fit` with the same budget and formats.
pub fn max_duration_ms(raw_budget_bytes: u64, formats: &[AudioFormat]) -> u64 {
    let mut best = 0u64;
    for f in formats.iter().filter(|f| !f.is_lossless()) {
        let Some(floor) = f.floor() else { continue };
        // Solve (B - fixed - per_s * d) * 8 * FILL >= floor * d for d.
        let (fixed, per_s) = match f {
            AudioFormat::OggOpus => ((raw_budget_bytes as f64 * 0.01).ceil() + 1024.0, 0.0),
            AudioFormat::Mp3 => (200.0, 0.0),
            AudioFormat::M4aAac => (8192.0, 40.0),
            AudioFormat::Flac | AudioFormat::Wav => continue,
        };
        let avail = raw_budget_bytes as f64 - fixed;
        if avail <= 0.0 {
            continue;
        }
        let d = avail * 8.0 * FILL / (floor.bitrate_bps as f64 + per_s * 8.0 * FILL);
        let mut ms = (d * 1000.0).floor() as u64;
        // Guard against rounding: step down until the floor rung really fits.
        while ms > 0 {
            let target = budget_bits(*f, ms, raw_budget_bytes) / (ms as f64 / 1000.0);
            if target >= floor.bitrate_bps as f64 {
                break;
            }
            ms -= 1;
        }
        best = best.max(ms);
    }
    best
}

/// After an overshoot, scale the bitrate by `budget / actual * 0.97`. Returns
/// `None` when no retry is warranted (it fit), when the format is lossless,
/// when the attempt limit is reached, or when the scaled rate drops below the
/// format floor.
pub fn retry_scale(plan: &AudioPlan, actual_bytes: u64, budget_bytes: u64) -> Option<AudioPlan> {
    if actual_bytes <= budget_bytes || plan.format.is_lossless() || actual_bytes == 0 {
        return None;
    }
    if plan.attempt >= MAX_ATTEMPTS {
        return None;
    }
    let scale = budget_bytes as f64 / actual_bytes as f64 * RETRY_MARGIN;
    let new_bps = (plan.bitrate_bps as f64 * scale).floor() as u32;
    let floor = plan.format.floor()?;
    if new_bps < floor.bitrate_bps {
        return None;
    }
    // Keep channel count and sample rate consistent with the ladder at the new rate.
    let rung = plan.ladder_rung_at_or_below(new_bps).unwrap_or(floor);
    let duration_ms = plan.implied_duration_ms();
    Some(AudioPlan {
        format: plan.format,
        bitrate_bps: new_bps,
        channels: rung.channels.min(plan.channels),
        sample_rate: rung.sample_rate,
        predicted_bytes: predict_bytes(plan.format, new_bps, duration_ms),
        attempt: plan.attempt + 1,
    })
}

impl AudioPlan {
    fn ladder_rung_at_or_below(&self, bps: u32) -> Option<Rung> {
        self.format
            .ladder()
            .iter()
            .find(|r| r.bitrate_bps <= bps)
            .copied()
    }

    /// Duration implied by predicted bytes and bitrate (used by retry to
    /// re-predict without the caller re-passing the duration).
    fn implied_duration_ms(&self) -> u64 {
        if self.bitrate_bps == 0 {
            return 0;
        }
        // Invert predict_bytes approximately: payload ~ predicted - overhead.
        let mut lo = 0u64;
        let mut hi = 1u64 << 40;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if predict_bytes(self.format, self.bitrate_bps, mid) < self.predicted_bytes {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// Smaller-mode decision for one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmallerDecision {
    /// WAV, AIFF, CAF PCM, ALAC: lossless to FLAC level 8 (roughly half).
    ToFlac,
    /// FLAC input: re-encode at level 8, keep only if 3 percent smaller.
    ReencodeFlac,
    /// Lossy input in Keep-quality mode: do nothing.
    KeepOriginal,
    /// Smallest files: Opus 96 kb/s stereo, or 48 kb/s for mono sources.
    ToOpus { bps: u32, channels: u16 },
}

/// DESIGN.md 3.6, "Smaller mode for audio".
pub fn plan_smaller(info: &AudioInfo, level: SmallerLevel) -> SmallerDecision {
    match level {
        SmallerLevel::KeepQuality => {
            if info.codec == "flac" {
                SmallerDecision::ReencodeFlac
            } else if info.lossless {
                SmallerDecision::ToFlac
            } else {
                SmallerDecision::KeepOriginal
            }
        }
        SmallerLevel::Smallest => {
            let (bps, channels) = if info.channels <= 1 {
                (48_000, 1)
            } else {
                (96_000, 2)
            };
            // An Opus or other lossy file already at or below the target
            // would only grow.
            if !info.lossless {
                if let Some(b) = info.bitrate_bps {
                    if b <= bps as u64 {
                        return SmallerDecision::KeepOriginal;
                    }
                }
            }
            SmallerDecision::ToOpus { bps, channels }
        }
    }
}
