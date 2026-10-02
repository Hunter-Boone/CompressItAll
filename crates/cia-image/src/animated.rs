//! Animated images (DESIGN.md 3.4.8): GIF in, GIF out. Fit steps in order:
//! fewer colours (256, 128, 64), drop every second frame when above 15 fps,
//! downscale, refuse below a 240 px long edge. Animated WebP output and the
//! MP4 path belong to the engine (video planner) and a later milestone.

use crate::{CancelFn, ImageError, ProgressFn};
use image::AnimationDecoder;
use std::io::Cursor;

#[derive(Debug, Clone)]
pub struct Frame {
    pub rgba: Vec<u8>,
    /// Delay in milliseconds.
    pub delay_ms: u32,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<Frame>,
    pub source_bytes: u64,
}

impl Animation {
    pub fn duration_ms(&self) -> u64 {
        self.frames.iter().map(|f| f.delay_ms as u64).sum()
    }
    pub fn fps(&self) -> f32 {
        let d = self.duration_ms();
        if d == 0 { 0.0 } else { self.frames.len() as f32 * 1000.0 / d as f32 }
    }
}

/// Decode a GIF (or APNG / animated WebP via `image`) into full coalesced frames.
pub fn decode_animation(bytes: &[u8]) -> Result<Animation, ImageError> {
    let frames: Vec<image::Frame> = match crate::decode::sniff(bytes) {
        crate::SourceFormat::Gif => image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).map_err(|e| ImageError::Damaged(e.to_string()))?.into_frames().collect_frames().map_err(|e| ImageError::Damaged(e.to_string()))?,
        crate::SourceFormat::Png => image::codecs::png::PngDecoder::new(Cursor::new(bytes)).map_err(|e| ImageError::Damaged(e.to_string()))?.apng().map_err(|e| ImageError::Damaged(e.to_string()))?.into_frames().collect_frames().map_err(|e| ImageError::Damaged(e.to_string()))?,
        crate::SourceFormat::Webp => image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).map_err(|e| ImageError::Damaged(e.to_string()))?.into_frames().collect_frames().map_err(|e| ImageError::Damaged(e.to_string()))?,
        other => return Err(ImageError::Unsupported(format!("animated {}", other.token()))),
    };
    let first = frames.first().ok_or_else(|| ImageError::Damaged("no frames".into()))?;
    let (width, height) = (first.buffer().width(), first.buffer().height());
    let frames = frames
        .into_iter()
        .map(|f| {
            let (num, den) = f.delay().numer_denom_ms();
            Frame { rgba: f.into_buffer().into_raw(), delay_ms: num.checked_div(den).map_or(100, |d| d.max(10)) }
        })
        .collect();
    Ok(Animation { width, height, frames, source_bytes: bytes.len() as u64 })
}

#[derive(Debug, Clone)]
pub struct AnimOptions {
    pub budget_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AnimResult {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub frames: usize,
    pub colours: u16,
    pub frame_dropped: bool,
    pub label: cia_core::QualityLabel,
}

#[derive(Debug, Clone)]
pub enum AnimOutcome {
    Encoded(AnimResult),
    KeptOriginal,
    Refused { smallest_bytes: u64 },
}

/// Re-encode as GIF: per-frame palette (quantette), unchanged pixels become transparent between frames.
pub fn encode_gif(anim: &Animation, colours: u16, drop_half: bool, scale_long_edge: Option<u32>) -> Result<Vec<u8>, ImageError> {
    let (w, h) = match scale_long_edge {
        Some(le) if le < anim.width.max(anim.height) => crate::resize::dims_for_long_edge(anim.width, anim.height, le),
        _ => (anim.width, anim.height),
    };
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w as u16, h as u16, &[]).map_err(|e| ImageError::Encoder(e.to_string()))?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| ImageError::Encoder(e.to_string()))?;
        let mut prev: Option<Vec<u8>> = None;
        let mut carry_delay = 0u32;
        for (i, f) in anim.frames.iter().enumerate() {
            if drop_half && i % 2 == 1 {
                carry_delay += f.delay_ms;
                continue;
            }
            let rgba = if (w, h) != (anim.width, anim.height) {
                let d = crate::decode::DecodedImage { width: anim.width, height: anim.height, rgba: f.rgba.clone(), has_alpha: true, source: crate::SourceFormat::Gif, source_bytes: 0, icc: None, exif: None, was_16_bit: false };
                crate::resize::downscale(&d, w, h, false).rgba
            } else {
                f.rgba.clone()
            };
            // Flatten alpha onto the previous frame (or black) before quantising.
            let mut flat = rgba.clone();
            if let Some(p) = &prev {
                for (px, pp) in flat.chunks_exact_mut(4).zip(p.chunks_exact(4)) {
                    if px[3] < 128 {
                        px.copy_from_slice(pp);
                    }
                }
            }
            let rgb: Vec<u8> = flat.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            let (pal, idx) = crate::encode::quantise_rgb(&rgb, w, h, colours.clamp(3, 256) - 1)?;
            let transparent = pal.len() as u8; // extra palette slot for "unchanged"
            let mut palette: Vec<u8> = pal.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
            palette.extend_from_slice(&[0, 0, 0]);
            let mut pixels = idx.clone();
            if let Some(p) = &prev {
                for (k, (px, pp)) in flat.chunks_exact(4).zip(p.chunks_exact(4)).enumerate() {
                    if px[..3] == pp[..3] {
                        pixels[k] = transparent;
                    }
                }
            }
            let delay = ((f.delay_ms + carry_delay) / 10).max(2) as u16;
            carry_delay = 0;
            let mut frame = gif::Frame { width: w as u16, height: h as u16, delay, palette: Some(palette), buffer: std::borrow::Cow::Owned(pixels), transparent: if prev.is_some() { Some(transparent) } else { None }, dispose: gif::DisposalMethod::Keep, ..Default::default() };
            frame.make_lzw_pre_encoded();
            enc.write_lzw_pre_encoded_frame(&frame).map_err(|e| ImageError::Encoder(e.to_string()))?;
            prev = Some(flat);
        }
    }
    Ok(out)
}

pub fn compress_animation(anim: &Animation, opts: &AnimOptions, cancel: CancelFn, progress: ProgressFn) -> Result<AnimOutcome, ImageError> {
    let budget = opts.budget_bytes;
    let mut smallest = anim.source_bytes;
    let mut long_edge = anim.width.max(anim.height);
    let can_drop = anim.fps() > 15.0;
    let steps: Vec<(u16, bool, Option<u32>)> = {
        let mut v = vec![(256u16, false, None), (128, false, None), (64, false, None)];
        if can_drop {
            v.push((256, true, None));
            v.push((128, true, None));
            v.push((64, true, None));
        }
        v
    };
    let mut best: Option<AnimResult> = None;
    let total = steps.len() as f32 + 3.0;
    let mut n = 0f32;
    let mut attempt = |colours: u16, drop: bool, le: Option<u32>| -> Result<Option<AnimResult>, ImageError> {
        if cancel() {
            return Err(ImageError::Cancelled);
        }
        n += 1.0;
        progress(n / total, "Re-encoding animation");
        let bytes = encode_gif(anim, colours, drop, le)?;
        let (w, h) = le.map(|l| crate::resize::dims_for_long_edge(anim.width, anim.height, l)).unwrap_or((anim.width, anim.height));
        let frames = if drop { anim.frames.len().div_ceil(2) } else { anim.frames.len() };
        let label = if colours >= 256 && !drop && le.is_none() { cia_core::QualityLabel::Great } else if colours >= 128 { cia_core::QualityLabel::Good } else { cia_core::QualityLabel::Okay };
        Ok(Some(AnimResult { bytes, width: w, height: h, frames, colours, frame_dropped: drop, label }))
    };
    for (colours, drop, le) in steps {
        if let Some(r) = attempt(colours, drop, le)? {
            smallest = smallest.min(r.bytes.len() as u64);
            match budget {
                None => {
                    if (r.bytes.len() as u64) * 100 <= anim.source_bytes * 95 {
                        return Ok(AnimOutcome::Encoded(r));
                    }
                    return Ok(AnimOutcome::KeptOriginal);
                }
                Some(b) if (r.bytes.len() as u64) < b => return Ok(AnimOutcome::Encoded(r)),
                _ => best = Some(r),
            }
        }
    }
    // Downscale rounds.
    for _ in 0..3 {
        let Some(b) = budget else { break };
        let scale = (b as f64 / smallest as f64).sqrt() * 0.97;
        long_edge = ((long_edge as f64) * scale).floor() as u32;
        if long_edge < 240 {
            break;
        }
        if let Some(r) = attempt(64, can_drop, Some(long_edge))? {
            smallest = smallest.min(r.bytes.len() as u64);
            if (r.bytes.len() as u64) < b {
                return Ok(AnimOutcome::Encoded(r));
            }
            best = Some(r);
        }
    }
    let _ = best;
    Ok(AnimOutcome::Refused { smallest_bytes: smallest })
}
