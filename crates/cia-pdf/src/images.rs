//! Step 3 of the PDF planner: find the image XObjects we are allowed to touch,
//! decode them once, and re-encode them at the ladder's `(quality, cap)`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::codec::{self, Predictor};
use crate::doc::{
    entry, entry_i64, entry_name, filters, first_decode_parms, is_image_xobject, plain_bytes,
    resolve,
};
use crate::resize::{capped_dims, downscale};
use crate::{JpegEncoder, PdfError};

/// How the image's samples are stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `/DCTDecode`: a baseline or progressive JPEG.
    Dct,
    /// Raw samples, optionally Flate-compressed and predictor-filtered.
    Raw { predictor: Option<Predictor> },
}

/// An image XObject the image pass may rewrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageTarget {
    pub id: ObjectId,
    pub source: Source,
    pub width: u32,
    pub height: u32,
    /// 1 (gray) or 3 (RGB).
    pub channels: u8,
    pub smask: Option<ObjectId>,
}

#[derive(Debug, Default)]
pub struct Collected {
    pub targets: Vec<ImageTarget>,
    /// Image XObjects we leave untouched (JPX, JBIG2, CCITT, Indexed, CMYK,
    /// 16-bit, masks, colour-key masked, ...).
    pub ineligible: usize,
}

impl Collected {
    pub fn total(&self) -> usize {
        self.targets.len() + self.ineligible
    }
}

/// Walk every stream in the document (which covers images nested in Form
/// XObjects, patterns and annotation appearances alike) and classify it.
pub fn collect_images(doc: &Document) -> Collected {
    let mut images: Vec<(ObjectId, &Stream)> = Vec::new();
    let mut masks: HashSet<ObjectId> = HashSet::new();
    for (id, obj) in &doc.objects {
        if let Object::Stream(s) = obj {
            if is_image_xobject(&s.dict) {
                images.push((*id, s));
                for key in [b"SMask".as_slice(), b"Mask"] {
                    if let Ok(Object::Reference(m)) = s.dict.get(key) {
                        masks.insert(*m);
                    }
                }
            }
        }
    }
    let mut out = Collected::default();
    for (id, stream) in images {
        if masks.contains(&id) {
            out.ineligible += 1;
            continue;
        }
        match eligible(doc, id, stream) {
            Some(t) => out.targets.push(t),
            None => out.ineligible += 1,
        }
    }
    out
}

fn eligible(doc: &Document, id: ObjectId, stream: &Stream) -> Option<ImageTarget> {
    let d = &stream.dict;
    if matches!(entry(doc, d, b"ImageMask"), Some(Object::Boolean(true))) {
        return None;
    }
    let width = entry_i64(doc, d, b"Width")?;
    let height = entry_i64(doc, d, b"Height")?;
    if !(1..=65_535).contains(&width) || !(1..=65_535).contains(&height) {
        return None;
    }
    let f = filters(doc, stream);
    let source = match f
        .iter()
        .map(|n| n.as_slice())
        .collect::<Vec<_>>()
        .as_slice()
    {
        [b"DCTDecode"] => Source::Dct,
        [] => Source::Raw { predictor: None },
        [b"FlateDecode"] => {
            let predictor = first_decode_parms(doc, stream);
            if let Some(p) = predictor {
                let supported = p.is_identity()
                    || (p.predictor == 2 && p.bits_per_component == 8)
                    || (10..=15).contains(&p.predictor);
                if !supported {
                    return None;
                }
            }
            Source::Raw { predictor }
        }
        _ => return None,
    };
    let bpc =
        entry_i64(doc, d, b"BitsPerComponent").unwrap_or(if source == Source::Dct { 8 } else { 0 });
    if bpc != 8 {
        return None;
    }
    let channels = colour_channels(doc, entry(doc, d, b"ColorSpace")?)?;
    // Colour-key masking is defined on exact sample values; lossy re-encoding
    // would break it.
    if matches!(entry(doc, d, b"Mask"), Some(Object::Array(_))) {
        return None;
    }
    let smask = match d.get(b"SMask") {
        Ok(Object::Reference(m)) => {
            // A pre-multiplied (/Matte) mask must match the base image pixel
            // for pixel; we would rather not downscale those.
            if let Ok(Object::Stream(ms)) = doc.get_object(*m) {
                if ms.dict.has(b"Matte") {
                    return None;
                }
            }
            Some(*m)
        }
        Ok(Object::Null) | Err(_) => None,
        Ok(_) => return None,
    };
    Some(ImageTarget {
        id,
        source,
        width: width as u32,
        height: height as u32,
        channels,
        smask,
    })
}

/// 1 or 3 for the colour spaces we handle; `None` for everything else.
fn colour_channels(doc: &Document, cs: &Object) -> Option<u8> {
    match cs {
        Object::Name(n) => match n.as_slice() {
            b"DeviceRGB" | b"CalRGB" => Some(3),
            b"DeviceGray" | b"CalGray" => Some(1),
            _ => None,
        },
        Object::Array(items) => {
            let family = resolve(doc, items.first()?).as_name().ok()?;
            match family {
                b"CalRGB" => Some(3),
                b"CalGray" => Some(1),
                b"ICCBased" => {
                    let profile = resolve(doc, items.get(1)?);
                    let pd = match profile {
                        Object::Stream(s) => &s.dict,
                        Object::Dictionary(d) => d,
                        _ => return None,
                    };
                    match entry_i64(doc, pd, b"N")? {
                        1 => Some(1),
                        3 => Some(3),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Photo,
    Graphic,
}

#[derive(Debug, Clone)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub pixels: Vec<u8>,
    pub class: Class,
}

/// Decoded pixels kept across ladder steps, bounded by a byte budget. Images
/// that do not fit are decoded again on demand; images that failed once are
/// not retried.
pub struct DecodeCache {
    entries: HashMap<ObjectId, Result<Decoded, String>>,
    bytes: usize,
    budget: usize,
}

impl Default for DecodeCache {
    fn default() -> Self {
        Self::with_budget(384 * 1024 * 1024)
    }
}

impl DecodeCache {
    pub fn with_budget(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            budget,
        }
    }

    pub fn get(&mut self, doc: &Document, t: &ImageTarget) -> Result<Cow<'_, Decoded>, String> {
        if !self.entries.contains_key(&t.id) {
            let decoded = decode_target(doc, t);
            match &decoded {
                Ok(d) if self.bytes + d.pixels.len() <= self.budget => {
                    self.bytes += d.pixels.len();
                    self.entries.insert(t.id, decoded);
                }
                Ok(_) => return decoded.map(Cow::Owned),
                Err(_) => {
                    self.entries.insert(t.id, decoded);
                }
            }
        }
        match self.entries.get(&t.id).expect("inserted above") {
            Ok(d) => Ok(Cow::Borrowed(d)),
            Err(e) => Err(e.clone()),
        }
    }
}

fn decode_target(doc: &Document, t: &ImageTarget) -> Result<Decoded, String> {
    let stream = match doc.get_object(t.id) {
        Ok(Object::Stream(s)) => s,
        _ => return Err("image object vanished".into()),
    };
    match t.source {
        Source::Dct => decode_jpeg(&stream.content, t),
        Source::Raw { predictor } => {
            let raw = plain_bytes(doc, stream)?;
            let raw = match predictor {
                Some(p) if !p.is_identity() => codec::unpredict(&raw, p)?,
                _ => raw,
            };
            let expected = t.width as usize * t.height as usize * t.channels as usize;
            if raw.len() < expected {
                return Err(format!(
                    "image data is {} bytes, expected {expected}",
                    raw.len()
                ));
            }
            let mut pixels = raw;
            pixels.truncate(expected);
            let class = classify(&pixels, t.width, t.height, t.channels);
            Ok(Decoded {
                width: t.width,
                height: t.height,
                channels: t.channels,
                pixels,
                class,
            })
        }
    }
}

/// Decode a JPEG to interleaved RGB or gray, checking it agrees with what the
/// PDF dictionary claims about it.
pub fn decode_jpeg(bytes: &[u8], t: &ImageTarget) -> Result<Decoded, String> {
    let (width, height, channels, pixels) = decode_jpeg_raw(bytes)?;
    if channels != t.channels {
        return Err(format!(
            "JPEG has {channels} components but the PDF declares {}",
            t.channels
        ));
    }
    if width != t.width || height != t.height {
        return Err(format!(
            "JPEG is {width}x{height} but the PDF declares {}x{}",
            t.width, t.height
        ));
    }
    Ok(Decoded {
        width,
        height,
        channels,
        pixels,
        // DCT sources are always re-encoded as JPEG; the class is moot.
        class: Class::Photo,
    })
}

/// (width, height, channels, pixels). CMYK and YCCK are refused.
pub fn decode_jpeg_raw(bytes: &[u8]) -> Result<(u32, u32, u8, Vec<u8>), String> {
    let mut probe = JpegDecoder::new(ZCursor::new(bytes));
    probe
        .decode_headers()
        .map_err(|e| format!("JPEG header: {e}"))?;
    let input = probe.input_colorspace().ok_or("JPEG colourspace unknown")?;
    let out_cs = match input {
        ColorSpace::Luma => ColorSpace::Luma,
        ColorSpace::YCbCr | ColorSpace::RGB => ColorSpace::RGB,
        other => return Err(format!("JPEG colourspace {other:?} is left untouched")),
    };
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(out_cs)
        .set_strict_mode(false);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    let pixels = dec.decode().map_err(|e| format!("JPEG decode: {e}"))?;
    let info = dec.info().ok_or("JPEG info missing after decode")?;
    let channels = out_cs.num_components() as u8;
    let (w, h) = (info.width as u32, info.height as u32);
    if pixels.len() < w as usize * h as usize * channels as usize {
        return Err("JPEG decoded to fewer bytes than its size implies".into());
    }
    Ok((w, h, channels, pixels))
}

/// Photo-or-Graphic in the spirit of DESIGN.md 3.4.2: measured on a thumbnail
/// no larger than 512 px, `Graphic` when there are at most 4096 distinct
/// colours or at least half of the 8x8 blocks are flat.
pub fn classify(pixels: &[u8], width: u32, height: u32, channels: u8) -> Class {
    let long = width.max(height);
    let step = long.div_ceil(512).max(1) as usize;
    let tw = (width as usize).div_ceil(step);
    let th = (height as usize).div_ceil(step);
    let c = channels as usize;
    let mut colours: HashSet<u32> = HashSet::new();
    let mut luma = vec![0u8; tw * th];
    for ty in 0..th {
        for tx in 0..tw {
            let sx = tx * step;
            let sy = ty * step;
            let i = (sy * width as usize + sx) * c;
            let (key, y) = if c == 3 {
                let (r, g, b) = (pixels[i], pixels[i + 1], pixels[i + 2]);
                (
                    (r as u32) << 16 | (g as u32) << 8 | b as u32,
                    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8,
                )
            } else {
                (pixels[i] as u32, pixels[i])
            };
            if colours.len() <= 4096 {
                colours.insert(key);
            }
            luma[ty * tw + tx] = y;
        }
    }
    if colours.len() <= 4096 {
        return Class::Graphic;
    }
    let (bx, by) = (tw / 8, th / 8);
    if bx == 0 || by == 0 {
        return Class::Photo;
    }
    let mut flat = 0usize;
    for j in 0..by {
        for i in 0..bx {
            let (mut lo, mut hi) = (255u8, 0u8);
            for y in 0..8 {
                let row = &luma[(j * 8 + y) * tw + i * 8..(j * 8 + y) * tw + i * 8 + 8];
                for &v in row {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            if hi - lo <= 2 {
                flat += 1;
            }
        }
    }
    if flat * 2 >= bx * by {
        Class::Graphic
    } else {
        Class::Photo
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PassStats {
    pub recompressed: usize,
    pub skipped: usize,
}

/// Rewrite every target at `(quality, cap)`, keeping a new stream only when
/// it is smaller than the one already in `doc`.
pub fn image_pass(
    doc: &mut Document,
    targets: &[ImageTarget],
    quality: u8,
    cap: Option<u32>,
    enc: &dyn JpegEncoder,
    cache: &mut DecodeCache,
    cancel: &dyn Fn() -> bool,
) -> Result<PassStats, PdfError> {
    let mut stats = PassStats::default();
    for t in targets {
        if cancel() {
            return Err(PdfError::Cancelled);
        }
        let decoded = match cache.get(doc, t) {
            Ok(d) => d,
            Err(e) => {
                log::debug!("image {:?} skipped: {e}", t.id);
                stats.skipped += 1;
                continue;
            }
        };
        let (w, h, pixels): (u32, u32, Cow<[u8]>) =
            match capped_dims(decoded.width, decoded.height, cap) {
                Some((nw, nh)) => match downscale(
                    &decoded.pixels,
                    decoded.width,
                    decoded.height,
                    decoded.channels,
                    nw,
                    nh,
                ) {
                    Ok(p) => (nw, nh, Cow::Owned(p)),
                    Err(e) => {
                        log::debug!("image {:?} resize failed: {e}", t.id);
                        stats.skipped += 1;
                        continue;
                    }
                },
                None => (
                    decoded.width,
                    decoded.height,
                    Cow::Borrowed(decoded.pixels.as_slice()),
                ),
            };
        let downscaled = w != decoded.width || h != decoded.height;
        let is_graphic_raw =
            matches!(t.source, Source::Raw { .. }) && decoded.class == Class::Graphic;
        let channels = decoded.channels;

        let (new_bytes, filter): (Vec<u8>, &[u8]) = if is_graphic_raw {
            if !downscaled {
                // Already re-deflated by the lossless pass; nothing lossy to do.
                stats.skipped += 1;
                continue;
            }
            (codec::deflate_best(&pixels), b"FlateDecode")
        } else {
            let encoded = if channels == 3 {
                enc.encode_rgb(&pixels, w, h, quality)
            } else {
                enc.encode_gray(&pixels, w, h, quality)
            };
            match encoded {
                Ok(j) => (j, b"DCTDecode"),
                Err(e) => {
                    log::debug!("image {:?} JPEG encode failed: {e}", t.id);
                    stats.skipped += 1;
                    continue;
                }
            }
        };

        let stream = match doc.get_object_mut(t.id) {
            Ok(Object::Stream(s)) => s,
            _ => {
                stats.skipped += 1;
                continue;
            }
        };
        if new_bytes.len() >= stream.content.len() {
            stats.skipped += 1;
            continue;
        }
        replace_stream(stream, new_bytes, filter, w, h);
        stats.recompressed += 1;
        if downscaled {
            if let Some(m) = t.smask {
                downscale_smask(doc, m, w, h);
            }
        }
    }
    Ok(stats)
}

fn replace_stream(stream: &mut Stream, bytes: Vec<u8>, filter: &[u8], w: u32, h: u32) {
    stream.dict.remove(b"DecodeParms");
    stream.dict.set("Filter", Object::Name(filter.to_vec()));
    stream.dict.set("Width", Object::Integer(w as i64));
    stream.dict.set("Height", Object::Integer(h as i64));
    stream.dict.set("BitsPerComponent", Object::Integer(8));
    stream.set_content(bytes);
}

/// Best effort: shrink an 8-bit DeviceGray Flate soft mask to the base
/// image's new size. Anything unusual leaves the mask as it was, which is
/// still valid: a soft mask may have different dimensions from its base.
fn downscale_smask(doc: &mut Document, id: ObjectId, base_w: u32, base_h: u32) {
    let Some((pixels, w, h)) = read_gray_mask(doc, id) else {
        return;
    };
    if w <= base_w && h <= base_h {
        return;
    }
    let Ok(small) = downscale(&pixels, w, h, 1, base_w, base_h) else {
        return;
    };
    let z = codec::deflate_best(&small);
    if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
        if z.len() < s.content.len() {
            replace_stream(s, z, b"FlateDecode", base_w, base_h);
        }
    }
}

fn read_gray_mask(doc: &Document, id: ObjectId) -> Option<(Vec<u8>, u32, u32)> {
    let stream = match doc.get_object(id) {
        Ok(Object::Stream(s)) => s,
        _ => return None,
    };
    let d: &Dictionary = &stream.dict;
    if entry_name(doc, d, b"ColorSpace") != Some(b"DeviceGray")
        || entry_i64(doc, d, b"BitsPerComponent") != Some(8)
    {
        return None;
    }
    let w = entry_i64(doc, d, b"Width")?;
    let h = entry_i64(doc, d, b"Height")?;
    if w <= 0 || h <= 0 {
        return None;
    }
    let f = filters(doc, stream);
    if !(f.is_empty() || f == [b"FlateDecode".to_vec()]) {
        return None;
    }
    let raw = plain_bytes(doc, stream).ok()?;
    let raw = match first_decode_parms(doc, stream) {
        Some(p) if !p.is_identity() => codec::unpredict(&raw, p).ok()?,
        _ => raw,
    };
    let expected = w as usize * h as usize;
    if raw.len() < expected {
        return None;
    }
    let mut px = raw;
    px.truncate(expected);
    Some((px, w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_image_is_graphic_and_noise_is_photo() {
        let flat = vec![128u8; 300 * 200 * 3];
        assert_eq!(classify(&flat, 300, 200, 3), Class::Graphic);
        // Deterministic pseudo-noise with far more than 4096 colours.
        let mut x: u32 = 12345;
        let noise: Vec<u8> = (0..300 * 200 * 3)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x & 0xff) as u8
            })
            .collect();
        assert_eq!(classify(&noise, 300, 200, 3), Class::Photo);
        let gray_flat = vec![7u8; 1000 * 700];
        assert_eq!(classify(&gray_flat, 1000, 700, 1), Class::Graphic);
    }
}
