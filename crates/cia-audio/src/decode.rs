//! Probe and decode with Symphonia (plus libopus for Ogg Opus when the
//! `opus-native` feature is on).

use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::*;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::{frames_to_ms, AudioError, Decoded};

/// What `probe` learned about an input without decoding it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// Lower-case codec id: "mp3", "flac", "vorbis", "opus", "pcm", "aac", "alac", "wma".
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub duration_ms: u64,
    /// Average bitrate over the whole file when the duration is known.
    pub bitrate_bps: Option<u64>,
    /// True for PCM, FLAC and ALAC.
    pub lossless: bool,
    /// AAC/M4A, ALAC and WMA: this crate cannot decode them; the host must use
    /// FFmpeg (desktop) or WebCodecs (web).
    pub needs_ffmpeg: bool,
}

const ASF_GUID: [u8; 16] = [
    0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C,
];

/// Containers we recognise by magic but deliberately do not decode.
fn detect_ffmpeg_only(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        // ISO BMFF: M4A/MP4/MOV. ALAC is tagged in the sample description.
        let head = &bytes[..bytes.len().min(1 << 20)];
        if head.windows(4).any(|w| w == b"alac") {
            return Some("alac");
        }
        return Some("aac");
    }
    if bytes.len() >= 16 && bytes[..16] == ASF_GUID {
        return Some("wma");
    }
    // Raw ADTS AAC: 0xFFF sync with layer bits 00.
    if bytes.len() >= 7 && bytes[0] == 0xFF && (bytes[1] & 0xF6) == 0xF0 {
        return Some("aac");
    }
    None
}

fn is_ogg_opus(bytes: &[u8]) -> bool {
    // First page: 27-byte header + one segment-table byte + "OpusHead".
    bytes.len() >= 36 && &bytes[0..4] == b"OggS" && bytes[28..36] == *b"OpusHead"
}

const PCM_TYPES: &[CodecType] = &[
    CODEC_TYPE_PCM_S32LE,
    CODEC_TYPE_PCM_S32LE_PLANAR,
    CODEC_TYPE_PCM_S32BE,
    CODEC_TYPE_PCM_S32BE_PLANAR,
    CODEC_TYPE_PCM_S24LE,
    CODEC_TYPE_PCM_S24LE_PLANAR,
    CODEC_TYPE_PCM_S24BE,
    CODEC_TYPE_PCM_S24BE_PLANAR,
    CODEC_TYPE_PCM_S16LE,
    CODEC_TYPE_PCM_S16LE_PLANAR,
    CODEC_TYPE_PCM_S16BE,
    CODEC_TYPE_PCM_S16BE_PLANAR,
    CODEC_TYPE_PCM_S8,
    CODEC_TYPE_PCM_S8_PLANAR,
    CODEC_TYPE_PCM_U32LE,
    CODEC_TYPE_PCM_U32LE_PLANAR,
    CODEC_TYPE_PCM_U32BE,
    CODEC_TYPE_PCM_U32BE_PLANAR,
    CODEC_TYPE_PCM_U24LE,
    CODEC_TYPE_PCM_U24LE_PLANAR,
    CODEC_TYPE_PCM_U24BE,
    CODEC_TYPE_PCM_U24BE_PLANAR,
    CODEC_TYPE_PCM_U16LE,
    CODEC_TYPE_PCM_U16LE_PLANAR,
    CODEC_TYPE_PCM_U16BE,
    CODEC_TYPE_PCM_U16BE_PLANAR,
    CODEC_TYPE_PCM_U8,
    CODEC_TYPE_PCM_U8_PLANAR,
    CODEC_TYPE_PCM_F32LE,
    CODEC_TYPE_PCM_F32LE_PLANAR,
    CODEC_TYPE_PCM_F32BE,
    CODEC_TYPE_PCM_F32BE_PLANAR,
    CODEC_TYPE_PCM_F64LE,
    CODEC_TYPE_PCM_F64LE_PLANAR,
    CODEC_TYPE_PCM_F64BE,
    CODEC_TYPE_PCM_F64BE_PLANAR,
];

fn codec_name(codec: CodecType) -> (&'static str, bool) {
    use symphonia::core::codecs::*;
    match codec {
        CODEC_TYPE_MP3 | CODEC_TYPE_MP2 | CODEC_TYPE_MP1 => ("mp3", false),
        CODEC_TYPE_FLAC => ("flac", true),
        CODEC_TYPE_VORBIS => ("vorbis", false),
        CODEC_TYPE_OPUS => ("opus", false),
        CODEC_TYPE_AAC => ("aac", false),
        CODEC_TYPE_ALAC => ("alac", true),
        CODEC_TYPE_WAVPACK => ("wavpack", true),
        CODEC_TYPE_ADPCM_IMA_WAV | CODEC_TYPE_ADPCM_MS | CODEC_TYPE_ADPCM_IMA_QT => {
            ("adpcm", false)
        }
        CODEC_TYPE_PCM_ALAW | CODEC_TYPE_PCM_MULAW => ("pcm", false),
        c if PCM_TYPES.contains(&c) => ("pcm", true),
        _ => ("unknown", false),
    }
}

struct Opened {
    format: Box<dyn FormatReader>,
    track_id: u32,
    params: CodecParameters,
}

fn open(bytes: &[u8]) -> Result<Opened, AudioError> {
    let owned: Vec<u8> = bytes.to_vec();
    let mss = MediaSourceStream::new(Box::new(Cursor::new(owned)), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Unsupported(format!("not a recognised audio file ({e})")))?;
    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Unsupported("no audio track".into()))?;
    Ok(Opened {
        track_id: track.id,
        params: track.codec_params.clone(),
        format,
    })
}

/// Sum packet durations (in frames) without decoding. Returns 0 if the
/// demuxer does not report per-packet durations.
fn count_frames_by_packets(format: &mut dyn FormatReader, track_id: u32) -> u64 {
    let mut frames = 0u64;
    while let Ok(p) = format.next_packet() {
        if p.track_id() == track_id {
            frames += p.dur();
        }
    }
    frames
}

/// Inspect an audio file: codec, channels, sample rate, duration, bitrate.
pub fn probe(bytes: &[u8]) -> Result<AudioInfo, AudioError> {
    if bytes.is_empty() {
        return Err(AudioError::Empty);
    }
    if let Some(codec) = detect_ffmpeg_only(bytes) {
        return Ok(AudioInfo {
            codec: codec.to_string(),
            channels: 0,
            sample_rate: 0,
            duration_ms: 0,
            bitrate_bps: None,
            lossless: codec == "alac",
            needs_ffmpeg: true,
        });
    }
    if is_ogg_opus(bytes) {
        #[cfg(feature = "opus-native")]
        {
            return crate::opus_codec::probe_opus_ogg(bytes);
        }
        #[cfg(not(feature = "opus-native"))]
        {
            return Err(AudioError::Unsupported(
                "Opus input needs libopus (opus-native feature) or WebCodecs".into(),
            ));
        }
    }

    let Opened {
        mut format,
        track_id,
        params,
    } = open(bytes)?;
    let (codec, lossless) = codec_name(params.codec);
    if matches!(codec, "aac" | "alac") {
        return Ok(AudioInfo {
            codec: codec.to_string(),
            channels: params.channels.map(|c| c.count() as u16).unwrap_or(0),
            sample_rate: params.sample_rate.unwrap_or(0),
            duration_ms: 0,
            bitrate_bps: None,
            lossless,
            needs_ffmpeg: true,
        });
    }
    if codec == "unknown" {
        return Err(AudioError::Unsupported(format!(
            "codec {:?} is not supported",
            params.codec
        )));
    }
    let sample_rate = params
        .sample_rate
        .ok_or_else(|| AudioError::Decode("sample rate unknown".into()))?;
    let channels = params.channels.map(|c| c.count() as u16).unwrap_or(0);

    let mut frames = params.n_frames.unwrap_or(0);
    if frames == 0 {
        frames = count_frames_by_packets(format.as_mut(), track_id);
    }
    if frames == 0 {
        // Last resort: decode and count.
        let d = decode(bytes, None)?;
        frames = d.frames() as u64;
    }
    let duration_ms = frames_to_ms(frames, sample_rate);
    let bitrate_bps = (bytes.len() as u64 * 8 * 1000).checked_div(duration_ms);
    Ok(AudioInfo {
        codec: codec.to_string(),
        channels,
        sample_rate,
        duration_ms,
        bitrate_bps,
        lossless,
        needs_ffmpeg: false,
    })
}

/// Decode a whole file (or its first `max_seconds`) to interleaved f32.
pub fn decode(bytes: &[u8], max_seconds: Option<f32>) -> Result<Decoded, AudioError> {
    if bytes.is_empty() {
        return Err(AudioError::Empty);
    }
    if let Some(codec) = detect_ffmpeg_only(bytes) {
        return Err(AudioError::NeedsFfmpeg(codec.to_string()));
    }
    if is_ogg_opus(bytes) {
        #[cfg(feature = "opus-native")]
        {
            return crate::opus_codec::decode_opus_ogg(bytes, max_seconds);
        }
        #[cfg(not(feature = "opus-native"))]
        {
            return Err(AudioError::Unsupported(
                "Opus input needs libopus (opus-native feature) or WebCodecs".into(),
            ));
        }
    }

    let Opened {
        mut format,
        track_id,
        params,
    } = open(bytes)?;
    let (codec, _) = codec_name(params.codec);
    if matches!(codec, "aac" | "alac") {
        return Err(AudioError::NeedsFfmpeg(codec.to_string()));
    }
    if codec == "opus" {
        return Err(AudioError::Unsupported(
            "Opus is only supported in an Ogg container".into(),
        ));
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| AudioError::Unsupported(format!("no decoder for {codec}: {e}")))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate = params.sample_rate.unwrap_or(0);
    let mut channels = params.channels.map(|c| c.count() as u16).unwrap_or(0);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut max_frames: Option<usize> = None;
    let mut decode_errors = 0u32;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(SymError::DecodeError(_)) => {
                decode_errors += 1;
                if decode_errors > 64 {
                    return Err(AudioError::Decode("too many corrupt packets".into()));
                }
                continue;
            }
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            Err(SymError::DecodeError(_)) => {
                decode_errors += 1;
                if decode_errors > 64 {
                    return Err(AudioError::Decode("too many corrupt packets".into()));
                }
                continue;
            }
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(AudioError::Decode(e.to_string())),
        };
        let spec = *audio.spec();
        if sample_rate == 0 {
            sample_rate = spec.rate;
        }
        if channels == 0 {
            channels = spec.channels.count() as u16;
        }
        if max_frames.is_none() {
            if let Some(s) = max_seconds {
                max_frames = Some((s.max(0.0) as f64 * sample_rate as f64).round() as usize);
            }
        }
        if audio.frames() == 0 {
            continue;
        }
        let buf = match sample_buf.as_mut() {
            Some(b) if b.capacity() >= audio.frames() * spec.channels.count() => b,
            _ => sample_buf.insert(SampleBuffer::new(audio.capacity() as u64, spec)),
        };
        buf.copy_interleaved_ref(audio);
        samples.extend_from_slice(buf.samples());
        if let Some(max) = max_frames {
            if samples.len() / channels.max(1) as usize >= max {
                samples.truncate(max * channels as usize);
                break;
            }
        }
    }

    if channels == 0 || sample_rate == 0 || samples.is_empty() {
        return Err(AudioError::Decode(if decode_errors > 0 {
            "no decodable audio".to_string()
        } else {
            "file contains no audio samples".to_string()
        }));
    }
    Ok(Decoded::new(sample_rate, channels, samples, codec))
}
