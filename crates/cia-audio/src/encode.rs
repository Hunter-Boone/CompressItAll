//! Lossless encoders: WAV (hound) and FLAC (flacenc, pure Rust, single-threaded).
//!
//! Both write 16-bit integer PCM. The engine works in f32 internally; samples
//! are rounded to the nearest 16-bit value and clamped.

use flacenc::component::BitRepr;
use flacenc::error::Verify;

use crate::{AudioError, Decoded};

pub(crate) const BITS: u32 = 16;

/// f32 in -1..1 to i16 with rounding and clamping.
pub(crate) fn to_i16(s: f32) -> i16 {
    let v = (s as f64 * 32768.0).round();
    v.clamp(-32768.0, 32767.0) as i16
}

/// 16-bit PCM WAV.
pub fn encode_wav(d: &Decoded) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: d.channels,
        sample_rate: d.sample_rate,
        bits_per_sample: BITS as u16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::with_capacity(44 + d.samples.len() * 2));
    {
        let mut w = hound::WavWriter::new(&mut cursor, spec).expect("in-memory wav writer");
        let mut sw = w.get_i16_writer(d.samples.len() as u32);
        for &s in &d.samples {
            sw.write_sample(to_i16(s));
        }
        sw.flush().expect("in-memory wav write");
        w.finalize().expect("in-memory wav finalize");
    }
    cursor.into_inner()
}

/// Map a libFLAC-style compression level (0..=8) onto flacenc settings.
fn flac_config(level: u8) -> flacenc::config::Encoder {
    let mut cfg = flacenc::config::Encoder::default();
    cfg.multithread = false;
    cfg.workers = None;
    cfg.block_size = 4096;
    let level = level.min(8);
    cfg.subframe_coding.qlpc.lpc_order = match level {
        0..=2 => 6,
        3..=5 => 8,
        6..=7 => 10,
        _ => 12,
    };
    cfg.subframe_coding.qlpc.quant_precision = if level >= 6 { 15 } else { 12 };
    let full_stereo = level >= 3;
    cfg.stereo_coding.use_leftside = full_stereo;
    cfg.stereo_coding.use_rightside = full_stereo;
    cfg.stereo_coding.use_midside = true;
    cfg
}

/// FLAC at the given compression level (8 is what the planner uses).
pub fn encode_flac(d: &Decoded, level: u8) -> Result<Vec<u8>, AudioError> {
    if d.channels == 0 || d.channels > 8 {
        return Err(AudioError::InvalidArgument(format!(
            "FLAC supports 1 to 8 channels, got {}",
            d.channels
        )));
    }
    if d.samples.is_empty() {
        return Err(AudioError::Empty);
    }
    let config = flac_config(level)
        .into_verified()
        .map_err(|e| AudioError::Encode(format!("flac config: {e:?}")))?;
    let pcm: Vec<i32> = d.samples.iter().map(|&s| to_i16(s) as i32).collect();
    let source = flacenc::source::MemSource::from_samples(
        &pcm,
        d.channels as usize,
        BITS as usize,
        d.sample_rate as usize,
    );
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| AudioError::Encode(format!("flac: {e}")))?;
    // flacenc records the short final block as STREAMINFO min_block_size, but
    // the frames use fixed-blocksize numbering. The FLAC spec excludes the last
    // block from the minimum, and Symphonia treats min != max as a
    // variable-blocksize stream and then rejects every frame header. Write what
    // libFLAC and FFmpeg write: min == max == block size.
    let block_size = config.block_size.min(d.frames().max(16));
    stream
        .stream_info_mut()
        .set_block_sizes(block_size, block_size.max(config.block_size))
        .map_err(|e| AudioError::Encode(format!("flac stream info: {e}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| AudioError::Encode(format!("flac write: {e}")))?;
    Ok(sink.as_slice().to_vec())
}
