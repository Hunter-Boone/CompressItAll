//! Sample-rate conversion (rubato, windowed-sinc) and channel mapping.

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::Decoded;

const CHUNK: usize = 1024;

/// Resample to `to_rate` with a 128-tap windowed sinc. Output length is exactly
/// `round(frames * to_rate / from_rate)`, so the duration is preserved.
pub fn resample(d: &Decoded, to_rate: u32) -> Decoded {
    if d.sample_rate == to_rate || d.channels == 0 || d.samples.is_empty() || to_rate == 0 {
        let mut out = d.clone();
        if to_rate != 0 && d.samples.is_empty() {
            out.sample_rate = to_rate;
        }
        return out;
    }
    let ch = d.channels as usize;
    let in_frames = d.frames();
    let ratio = to_rate as f64 / d.sample_rate as f64;
    let want = (in_frames as f64 * ratio).round() as usize;

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: rubato::calculate_cutoff(128, WindowFunction::BlackmanHarris2),
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut rs = SincFixedIn::<f32>::new(ratio, 1.0, params, CHUNK, ch).expect("valid resampler");
    let delay = rs.output_delay();

    // De-interleave.
    let mut planar: Vec<Vec<f32>> = vec![Vec::with_capacity(in_frames); ch];
    for frame in d.samples.chunks_exact(ch) {
        for (c, &s) in frame.iter().enumerate() {
            planar[c].push(s);
        }
    }
    let mut out_planar: Vec<Vec<f32>> = vec![Vec::with_capacity(want + delay + CHUNK); ch];
    let mut pos = 0;
    while pos + CHUNK <= in_frames {
        let slices: Vec<&[f32]> = planar.iter().map(|p| &p[pos..pos + CHUNK]).collect();
        let got = rs.process(&slices, None).expect("resample chunk");
        for (c, v) in got.into_iter().enumerate() {
            out_planar[c].extend_from_slice(&v);
        }
        pos += CHUNK;
    }
    if pos < in_frames {
        let slices: Vec<&[f32]> = planar.iter().map(|p| &p[pos..]).collect();
        let got = rs
            .process_partial(Some(&slices), None)
            .expect("resample tail");
        for (c, v) in got.into_iter().enumerate() {
            out_planar[c].extend_from_slice(&v);
        }
    }
    // Flush the filter delay.
    while out_planar[0].len() < want + delay {
        let got = rs
            .process_partial::<Vec<f32>>(None, None)
            .expect("resample flush");
        if got[0].is_empty() {
            break;
        }
        for (c, v) in got.into_iter().enumerate() {
            out_planar[c].extend_from_slice(&v);
        }
    }

    let mut samples = Vec::with_capacity(want * ch);
    for i in delay..delay + want {
        for p in &out_planar {
            samples.push(p.get(i).copied().unwrap_or(0.0));
        }
    }
    Decoded::new(to_rate, d.channels, samples, &d.source_codec)
}

/// Average all channels into one.
pub fn downmix_mono(d: &Decoded) -> Decoded {
    if d.channels <= 1 {
        return d.clone();
    }
    let ch = d.channels as usize;
    let inv = 1.0 / ch as f32;
    let samples: Vec<f32> = d
        .samples
        .chunks_exact(ch)
        .map(|f| f.iter().sum::<f32>() * inv)
        .collect();
    Decoded::new(d.sample_rate, 1, samples, &d.source_codec)
}

/// Produce exactly `channels` channels: downmix to mono, duplicate mono to
/// stereo, or keep the first `channels` of a wider layout.
pub fn with_channels(d: &Decoded, channels: u16) -> Decoded {
    if channels == 0 || d.channels == channels {
        return d.clone();
    }
    if channels == 1 {
        return downmix_mono(d);
    }
    let ch = d.channels as usize;
    let want = channels as usize;
    let mut samples = Vec::with_capacity(d.frames() * want);
    if ch == 1 {
        for &s in &d.samples {
            samples.extend(std::iter::repeat_n(s, want));
        }
    } else {
        for f in d.samples.chunks_exact(ch) {
            for c in 0..want {
                samples.push(f.get(c).copied().unwrap_or(0.0));
            }
        }
    }
    Decoded::new(d.sample_rate, channels, samples, &d.source_codec)
}
