//! Byte-level codecs shared by the lossless pass, the image pass and the
//! verifier: strict zlib inflate, best-effort deflate (zopfli for small
//! streams, zlib-rs level 9 above that) and the PNG/TIFF predictors that
//! Flate image streams may carry in `/DecodeParms`.

use std::io::Read;
use std::num::NonZeroU64;

/// Streams at or above this size skip zopfli and use zlib-rs level 9.
pub const ZOPFLI_MAX: usize = 256 * 1024;

/// Hard cap on what a single stream may inflate to (256 MB). Anything larger
/// is treated as damaged rather than decompressed.
pub const INFLATE_LIMIT: usize = 256 * 1024 * 1024;

/// Strict zlib inflate: a checksum or framing error is an error, never a
/// partial result. `limit` bounds the output so a decompression bomb cannot
/// exhaust memory.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::read::ZlibDecoder::new(data).take(limit as u64 + 1);
    let mut out = Vec::with_capacity(data.len().saturating_mul(3).min(limit));
    decoder
        .read_to_end(&mut out)
        .map_err(|e| format!("zlib inflate failed: {e}"))?;
    if out.len() > limit {
        return Err(format!("stream inflates past the {limit} byte limit"));
    }
    Ok(out)
}

/// Deflate as small as we reasonably can: zopfli under [`ZOPFLI_MAX`], zlib-rs
/// level 9 otherwise. Always returns a zlib-framed stream.
pub fn deflate_best(data: &[u8]) -> Vec<u8> {
    let level9 = deflate_level9(data);
    if data.is_empty() || data.len() >= ZOPFLI_MAX {
        return level9;
    }
    let iterations = if data.len() < 32 * 1024 { 15 } else { 5 };
    let options = zopfli::Options {
        iteration_count: NonZeroU64::new(iterations).expect("non-zero"),
        ..Default::default()
    };
    let mut out = Vec::with_capacity(level9.len());
    match zopfli::compress(options, zopfli::Format::Zlib, data, &mut out) {
        Ok(()) if out.len() < level9.len() => out,
        Ok(()) => level9,
        Err(e) => {
            log::warn!("zopfli failed ({e}); using zlib level 9");
            level9
        }
    }
}

fn deflate_level9(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(data.len() / 2 + 64),
        flate2::Compression::new(9),
    );
    // Writing to a Vec cannot fail.
    enc.write_all(data).expect("in-memory deflate");
    enc.finish().expect("in-memory deflate")
}

/// Predictor parameters from a `/DecodeParms` dictionary, in the shape the
/// PDF spec defines for `FlateDecode` and `LZWDecode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Predictor {
    pub predictor: i64,
    pub colors: usize,
    pub bits_per_component: usize,
    pub columns: usize,
}

impl Predictor {
    pub fn is_identity(&self) -> bool {
        self.predictor <= 1
    }
}

/// Undo a PNG (10..=15) or TIFF (2) predictor. Only 8-bit TIFF prediction is
/// supported; the PNG variants work for every bit depth since they operate on
/// bytes. Returns an error for anything else so the caller leaves the image
/// alone.
pub fn unpredict(data: &[u8], p: Predictor) -> Result<Vec<u8>, String> {
    if p.is_identity() {
        return Ok(data.to_vec());
    }
    if p.colors == 0 || p.columns == 0 || p.bits_per_component == 0 {
        return Err("invalid predictor parameters".into());
    }
    let bpp = (p.colors * p.bits_per_component).div_ceil(8).max(1);
    let row_len = (p.colors * p.bits_per_component * p.columns).div_ceil(8);
    match p.predictor {
        2 => {
            if p.bits_per_component != 8 {
                return Err("TIFF predictor only supported at 8 bits".into());
            }
            let mut out = data.to_vec();
            for row in out.chunks_mut(row_len) {
                for i in bpp..row.len() {
                    row[i] = row[i].wrapping_add(row[i - bpp]);
                }
            }
            Ok(out)
        }
        10..=15 => {
            let stride = row_len + 1;
            if !data.len().is_multiple_of(stride) {
                return Err(format!(
                    "PNG-predicted data length {} is not a multiple of row stride {stride}",
                    data.len()
                ));
            }
            let rows = data.len() / stride;
            let mut out = vec![0u8; rows * row_len];
            let mut prev = vec![0u8; row_len];
            for r in 0..rows {
                let src = &data[r * stride..(r + 1) * stride];
                let filter = src[0];
                let src = &src[1..];
                let cur = &mut out[r * row_len..(r + 1) * row_len];
                for i in 0..row_len {
                    let a = if i >= bpp { cur[i - bpp] } else { 0 };
                    let b = prev[i];
                    let c = if i >= bpp { prev[i - bpp] } else { 0 };
                    let x = src[i];
                    cur[i] = match filter {
                        0 => x,
                        1 => x.wrapping_add(a),
                        2 => x.wrapping_add(b),
                        3 => x.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                        4 => x.wrapping_add(paeth(a, b, c)),
                        other => return Err(format!("unknown PNG filter type {other}")),
                    };
                }
                prev.copy_from_slice(cur);
            }
            Ok(out)
        }
        other => Err(format!("unsupported predictor {other}")),
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_inflate_round_trip() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let z = deflate_best(&data);
        assert!(z.len() < data.len());
        assert_eq!(inflate(&z, INFLATE_LIMIT).unwrap(), data);
        let big = vec![7u8; ZOPFLI_MAX + 10];
        assert_eq!(inflate(&deflate_best(&big), INFLATE_LIMIT).unwrap(), big);
        assert_eq!(
            inflate(&deflate_best(&[]), INFLATE_LIMIT).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn inflate_rejects_garbage_and_truncation() {
        assert!(inflate(b"not zlib at all", INFLATE_LIMIT).is_err());
        let z = deflate_best(&[1u8; 5000]);
        assert!(inflate(&z[..z.len() - 3], INFLATE_LIMIT).is_err());
        assert!(inflate(&z, 100).is_err());
    }

    #[test]
    fn png_predictors_round_trip() {
        // 3 columns, 2 colours, 8 bpc: rows of 6 bytes.
        let rows: [[u8; 6]; 3] = [
            [10, 20, 30, 40, 50, 60],
            [11, 22, 33, 44, 55, 66],
            [1, 2, 3, 4, 5, 6],
        ];
        let p = Predictor {
            predictor: 15,
            colors: 2,
            bits_per_component: 8,
            columns: 3,
        };
        // Hand-encode with a different filter per row.
        let mut enc = Vec::new();
        // Row 0: None
        enc.push(0);
        enc.extend_from_slice(&rows[0]);
        // Row 1: Up
        enc.push(2);
        for (cur, above) in rows[1].iter().zip(rows[0].iter()) {
            enc.push(cur.wrapping_sub(*above));
        }
        // Row 2: Sub
        enc.push(1);
        for i in 0..6 {
            let a = if i >= 2 { rows[2][i - 2] } else { 0 };
            enc.push(rows[2][i].wrapping_sub(a));
        }
        let out = unpredict(&enc, p).unwrap();
        assert_eq!(out, rows.concat());
    }

    #[test]
    fn tiff_predictor_round_trip() {
        let p = Predictor {
            predictor: 2,
            colors: 1,
            bits_per_component: 8,
            columns: 4,
        };
        let enc = [5u8, 1, 1, 1, 9, 0, 0, 1];
        assert_eq!(unpredict(&enc, p).unwrap(), vec![5, 6, 7, 8, 9, 9, 9, 10]);
    }
}
