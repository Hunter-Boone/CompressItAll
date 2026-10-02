//! Native Opus: libopus encode into an Ogg container (RFC 7845) and Ogg Opus
//! decode. Only built with the `opus-native` feature; the web host uses
//! WebCodecs instead.

use std::io::Cursor;

use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};

use crate::decode::AudioInfo;
use crate::{frames_to_ms, resample, with_channels, AudioError, Decoded};

pub const OPUS_RATE: u32 = 48_000;
/// 20 ms at 48 kHz.
pub const FRAME: usize = 960;
/// Packets per Ogg page (1 s), as opusenc does.
const PACKETS_PER_PAGE: usize = 50;
const VENDOR: &str = "Smidge cia-audio (libopus)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpusApplication {
    /// Speech-tuned; the planner uses it below 32 kb/s.
    Voip,
    /// General audio.
    Audio,
}

impl OpusApplication {
    fn raw(self) -> opus::Application {
        match self {
            OpusApplication::Voip => opus::Application::Voip,
            OpusApplication::Audio => opus::Application::Audio,
        }
    }
}

fn opus_channels(n: u16) -> Result<opus::Channels, AudioError> {
    match n {
        1 => Ok(opus::Channels::Mono),
        2 => Ok(opus::Channels::Stereo),
        _ => Err(AudioError::InvalidArgument(format!(
            "Opus output supports 1 or 2 channels, got {n}"
        ))),
    }
}

fn opus_head(channels: u16, pre_skip: u16, input_rate: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(19);
    h.extend_from_slice(b"OpusHead");
    h.push(1); // version
    h.push(channels as u8);
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&input_rate.to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes()); // output gain
    h.push(0); // channel mapping family 0 (mono/stereo)
    h
}

fn opus_tags() -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"OpusTags");
    t.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    t.extend_from_slice(VENDOR.as_bytes());
    let comment = format!("ENCODER=Smidge cia-audio; libopus {}", opus::version());
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(&(comment.len() as u32).to_le_bytes());
    t.extend_from_slice(comment.as_bytes());
    t
}

/// Encode to Ogg Opus. Resamples to 48 kHz, maps to `channels` (1 or 2),
/// encodes 20 ms frames, writes OpusHead/OpusTags with the encoder's
/// look-ahead as pre-skip and an end granule that trims the padded tail so a
/// decoder returns exactly the source duration.
pub fn encode_opus_ogg(
    d: &Decoded,
    bitrate_bps: u32,
    channels: u16,
    application: OpusApplication,
) -> Result<Vec<u8>, AudioError> {
    if d.samples.is_empty() || d.channels == 0 {
        return Err(AudioError::Empty);
    }
    if !(6_000..=510_000).contains(&bitrate_bps) {
        return Err(AudioError::InvalidArgument(format!(
            "Opus bitrate {bitrate_bps} b/s is outside 6k..510k"
        )));
    }
    let ch_enum = opus_channels(channels)?;
    let mapped = with_channels(d, channels);
    let pcm = resample(&mapped, OPUS_RATE);
    let ch = channels as usize;
    let total_frames = pcm.frames() as u64;

    let mut enc = opus::Encoder::new(OPUS_RATE, ch_enum, application.raw())
        .map_err(|e| AudioError::Encode(format!("opus encoder: {e}")))?;
    enc.set_bitrate(opus::Bitrate::Bits(bitrate_bps as i32))
        .map_err(|e| AudioError::Encode(format!("opus bitrate: {e}")))?;
    enc.set_vbr(true)
        .map_err(|e| AudioError::Encode(format!("opus vbr: {e}")))?;
    enc.set_vbr_constraint(true)
        .map_err(|e| AudioError::Encode(format!("opus cvbr: {e}")))?;
    enc.set_complexity(10)
        .map_err(|e| AudioError::Encode(format!("opus complexity: {e}")))?;
    let pre_skip = enc
        .get_lookahead()
        .map_err(|e| AudioError::Encode(format!("opus lookahead: {e}")))?
        .max(0) as u64;

    let serial = 0x5349_4d47 ^ (total_frames as u32).wrapping_mul(2654435761);
    let mut out = Cursor::new(Vec::with_capacity(
        (bitrate_bps as u64 * total_frames / OPUS_RATE as u64 / 8) as usize + 4096,
    ));
    {
        let mut w = PacketWriter::new(&mut out);
        w.write_packet(
            opus_head(
                channels,
                pre_skip.min(u16::MAX as u64) as u16,
                d.sample_rate,
            ),
            serial,
            PacketWriteEndInfo::EndPage,
            0,
        )
        .map_err(|e| AudioError::Encode(format!("ogg: {e}")))?;
        w.write_packet(opus_tags(), serial, PacketWriteEndInfo::EndPage, 0)
            .map_err(|e| AudioError::Encode(format!("ogg: {e}")))?;

        let end_granule = pre_skip + total_frames;
        // Enough frames to cover the input plus the look-ahead so the decoder
        // can reconstruct every source sample.
        let needed = total_frames + pre_skip;
        let n_packets = needed.div_ceil(FRAME as u64) as usize;
        let mut frame_buf = vec![0f32; FRAME * ch];
        let mut packet_buf = vec![0u8; 4000];
        let mut granule = 0u64;
        for i in 0..n_packets {
            let start = i * FRAME;
            frame_buf.iter_mut().for_each(|s| *s = 0.0);
            let avail = pcm.frames().saturating_sub(start).min(FRAME);
            if avail > 0 {
                frame_buf[..avail * ch]
                    .copy_from_slice(&pcm.samples[start * ch..(start + avail) * ch]);
            }
            let n = enc
                .encode_float(&frame_buf, &mut packet_buf)
                .map_err(|e| AudioError::Encode(format!("opus encode: {e}")))?;
            granule += FRAME as u64;
            let last = i + 1 == n_packets;
            let info = if last {
                PacketWriteEndInfo::EndStream
            } else if (i + 1) % PACKETS_PER_PAGE == 0 {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            let absgp = if last { end_granule } else { granule };
            w.write_packet(packet_buf[..n].to_vec(), serial, info, absgp)
                .map_err(|e| AudioError::Encode(format!("ogg: {e}")))?;
        }
    }
    Ok(out.into_inner())
}

struct Head {
    channels: u16,
    pre_skip: u64,
    input_rate: u32,
}

fn parse_head(p: &[u8]) -> Result<Head, AudioError> {
    if p.len() < 19 || &p[..8] != b"OpusHead" {
        return Err(AudioError::Decode("missing OpusHead".into()));
    }
    if p[8] >> 4 != 0 {
        return Err(AudioError::Unsupported(format!(
            "OpusHead version {}",
            p[8]
        )));
    }
    let channels = p[9] as u16;
    let pre_skip = u16::from_le_bytes([p[10], p[11]]) as u64;
    let input_rate = u32::from_le_bytes([p[12], p[13], p[14], p[15]]);
    let family = p[18];
    if family != 0 || !(1..=2).contains(&channels) {
        return Err(AudioError::Unsupported(format!(
            "Opus channel mapping family {family} with {channels} channels"
        )));
    }
    Ok(Head {
        channels,
        pre_skip,
        input_rate,
    })
}

fn read_err(e: ogg::OggReadError) -> AudioError {
    AudioError::Decode(format!("ogg: {e}"))
}

/// Probe an Ogg Opus file without decoding: walks the pages to the last
/// granule position.
pub fn probe_opus_ogg(bytes: &[u8]) -> Result<AudioInfo, AudioError> {
    let mut r = PacketReader::new(Cursor::new(bytes));
    let first = r
        .read_packet()
        .map_err(read_err)?
        .ok_or_else(|| AudioError::Decode("empty Ogg stream".into()))?;
    let head = parse_head(&first.data)?;
    let serial = first.stream_serial();
    let mut last_granule = 0u64;
    while let Ok(Some(p)) = r.read_packet() {
        if p.stream_serial() == serial {
            last_granule = last_granule.max(p.absgp_page());
        }
    }
    let frames = last_granule.saturating_sub(head.pre_skip);
    let duration_ms = frames_to_ms(frames, OPUS_RATE);
    Ok(AudioInfo {
        codec: "opus".into(),
        channels: head.channels,
        sample_rate: if head.input_rate == 0 {
            OPUS_RATE
        } else {
            head.input_rate
        },
        duration_ms,
        bitrate_bps: (bytes.len() as u64 * 8 * 1000).checked_div(duration_ms),
        lossless: false,
        needs_ffmpeg: false,
    })
}

/// Decode Ogg Opus with libopus to 48 kHz f32, honouring pre-skip and the
/// end trimming implied by the final granule position.
pub fn decode_opus_ogg(bytes: &[u8], max_seconds: Option<f32>) -> Result<Decoded, AudioError> {
    let mut r = PacketReader::new(Cursor::new(bytes));
    let first = r
        .read_packet()
        .map_err(read_err)?
        .ok_or_else(|| AudioError::Decode("empty Ogg stream".into()))?;
    let head = parse_head(&first.data)?;
    let serial = first.stream_serial();
    let ch = head.channels as usize;
    let tags = r
        .read_packet()
        .map_err(read_err)?
        .ok_or_else(|| AudioError::Decode("missing OpusTags".into()))?;
    if tags.data.len() < 8 || &tags.data[..8] != b"OpusTags" {
        return Err(AudioError::Decode("missing OpusTags".into()));
    }
    let mut dec = opus::Decoder::new(OPUS_RATE, opus_channels(head.channels)?)
        .map_err(|e| AudioError::Decode(format!("opus decoder: {e}")))?;
    let max_frames = max_seconds.map(|s| (s.max(0.0) as f64 * OPUS_RATE as f64).round() as u64);

    let mut pcm_buf = vec![0f32; 5760 * ch];
    let mut samples: Vec<f32> = Vec::new();
    let mut decoded_frames = 0u64; // including pre-skip
    let mut last_granule: Option<u64> = None;
    let mut errors = 0u32;
    loop {
        let p = match r.read_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(e) => {
                errors += 1;
                if errors > 8 {
                    return Err(read_err(e));
                }
                break;
            }
        };
        if p.stream_serial() != serial {
            continue;
        }
        let n = match dec.decode_float(&p.data, &mut pcm_buf, false) {
            Ok(n) => n,
            Err(_) => {
                errors += 1;
                if errors > 64 {
                    return Err(AudioError::Decode("too many corrupt Opus packets".into()));
                }
                continue;
            }
        };
        let frame_start = decoded_frames;
        decoded_frames += n as u64;
        // Drop the pre-skip region.
        let skip = head.pre_skip.saturating_sub(frame_start).min(n as u64) as usize;
        samples.extend_from_slice(&pcm_buf[skip * ch..n * ch]);
        if p.last_in_page() || p.last_in_stream() {
            last_granule = Some(p.absgp_page());
        }
        if let Some(max) = max_frames {
            if samples.len() as u64 / ch as u64 >= max {
                samples.truncate(max as usize * ch);
                last_granule = None;
                break;
            }
        }
    }
    if let Some(g) = last_granule {
        let keep = g.saturating_sub(head.pre_skip) as usize;
        if keep * ch < samples.len() {
            samples.truncate(keep * ch);
        }
    }
    if samples.is_empty() {
        return Err(AudioError::Decode("no decodable Opus audio".into()));
    }
    Ok(Decoded::new(OPUS_RATE, head.channels, samples, "opus"))
}
