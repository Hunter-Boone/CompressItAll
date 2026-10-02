//! One function per candidate, with the fixed settings from DESIGN.md 3.4.3.

use crate::candidates::Candidate;
use crate::classify::Class;
use crate::decode::DecodedImage;
use crate::ImageError;

pub struct EncodeInput<'a> {
    pub img: &'a DecodedImage,
    pub class: Class,
}

pub fn encode(input: &EncodeInput, candidate: Candidate, quality: u8) -> Result<Vec<u8>, ImageError> {
    let img = input.img;
    match candidate {
        Candidate::Jpeg => {
            let opts = if input.class == Class::Graphic { cia_mozjpeg::JpegOptions::graphic(quality) } else { cia_mozjpeg::JpegOptions::photo(quality) };
            let rgb = img.rgb();
            cia_mozjpeg::encode(&rgb, img.width, img.height, cia_mozjpeg::PixelLayout::Rgb, &opts).map_err(|e| ImageError::Encoder(e.to_string()))
        }
        Candidate::WebpLossy => {
            let opts = cia_webp::LossyOptions::planner(quality as f32);
            if img.has_alpha {
                cia_webp::encode_lossy(&img.rgba, img.width, img.height, true, &opts).map_err(|e| ImageError::Encoder(e.to_string()))
            } else {
                let rgb = img.rgb();
                cia_webp::encode_lossy(&rgb, img.width, img.height, false, &opts).map_err(|e| ImageError::Encoder(e.to_string()))
            }
        }
        Candidate::WebpLossless => {
            if img.has_alpha {
                cia_webp::encode_lossless(&img.rgba, img.width, img.height, true, 100).map_err(|e| ImageError::Encoder(e.to_string()))
            } else {
                let rgb = img.rgb();
                cia_webp::encode_lossless(&rgb, img.width, img.height, false, 100).map_err(|e| ImageError::Encoder(e.to_string()))
            }
        }
        Candidate::Avif => {
            let enc = ravif::Encoder::new().with_quality(quality as f32).with_alpha_quality(quality as f32).with_speed(6).with_num_threads(Some(1));
            let res = if img.has_alpha {
                let px: Vec<ravif::RGBA8> = img.rgba.chunks_exact(4).map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
                enc.encode_rgba(ravif::Img::new(px.as_slice(), img.width as usize, img.height as usize))
            } else {
                let px: Vec<ravif::RGB8> = img.rgba.chunks_exact(4).map(|p| ravif::RGB8::new(p[0], p[1], p[2])).collect();
                enc.encode_rgb(ravif::Img::new(px.as_slice(), img.width as usize, img.height as usize))
            };
            res.map(|e| e.avif_file).map_err(|e| ImageError::Encoder(e.to_string()))
        }
        Candidate::PngLossless => png_lossless(img),
        Candidate::PngPalette(colours) => png_palette(img, colours),
    }
}

/// Raw PNG (fast, unoptimised) then oxipng preset 4, strip safe, zopfli under 2 MP.
pub fn png_lossless(img: &DecodedImage) -> Result<Vec<u8>, ImageError> {
    let raw = raw_png(img.width, img.height, &img.rgba, img.has_alpha)?;
    oxipng_optimise(&raw, img.pixels() < 2_000_000)
}

pub fn oxipng_optimise(png: &[u8], zopfli: bool) -> Result<Vec<u8>, ImageError> {
    let mut opts = oxipng::Options::from_preset(4);
    opts.strip = oxipng::StripChunks::Safe;
    opts.optimize_alpha = true;
    if zopfli {
        opts.deflate = oxipng::Deflaters::Zopfli { iterations: std::num::NonZeroU8::new(15).unwrap() };
    }
    oxipng::optimize_from_memory(png, &opts).map_err(|e| ImageError::Encoder(e.to_string()))
}

fn raw_png(width: u32, height: u32, rgba: &[u8], has_alpha: bool) -> Result<Vec<u8>, ImageError> {
    let mut out = Vec::new();
    {
        let enc = image::codecs::png::PngEncoder::new_with_quality(&mut out, image::codecs::png::CompressionType::Fast, image::codecs::png::FilterType::NoFilter);
        use image::ImageEncoder;
        if has_alpha {
            enc.write_image(rgba, width, height, image::ExtendedColorType::Rgba8).map_err(|e| ImageError::Encoder(e.to_string()))?;
        } else {
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            enc.write_image(&rgb, width, height, image::ExtendedColorType::Rgb8).map_err(|e| ImageError::Encoder(e.to_string()))?;
        }
    }
    Ok(out)
}

/// Quantise packed RGB to at most `colours` colours (k-means, Floyd-Steinberg dithered).
pub fn quantise_rgb(rgb: &[u8], width: u32, height: u32, colours: u16) -> Result<(Vec<[u8; 3]>, Vec<u8>), ImageError> {
    use quantette::deps::palette::cast::from_component_slice;
    use quantette::deps::palette::Srgb;
    use quantette::dither::FloydSteinberg;
    use quantette::{ImageRef, PaletteSize, Pipeline, QuantizeMethod};
    let pixels: &[Srgb<u8>] = from_component_slice(rgb);
    let image = ImageRef::new(width, height, pixels).map_err(|e| ImageError::Encoder(e.to_string()))?;
    let size = PaletteSize::try_from(colours.clamp(2, 256)).map_err(|e| ImageError::Encoder(e.to_string()))?;
    let indexed = Pipeline::new().palette_size(size).quantize_method(QuantizeMethod::kmeans()).ditherer(FloydSteinberg::new()).input_image(image).output_srgb8_indexed_image();
    let (pal, idx) = indexed.into_parts();
    Ok((pal.iter().map(|c| [c.red, c.green, c.blue]).collect(), idx))
}

/// Palette-quantised PNG via quantette (k-means, dithered), then oxipng.
pub fn png_palette(img: &DecodedImage, colours: u16) -> Result<Vec<u8>, ImageError> {
    let rgba = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone()).ok_or_else(|| ImageError::Encoder("buffer".into()))?;
    let (palette, indices): (Vec<[u8; 4]>, Vec<u8>) = if img.has_alpha {
        // quantette works on RGB; quantise colour, then carry alpha per pixel bucketed to 16 levels.
        let (pal, idx) = quantise_rgb(&img.rgb(), img.width, img.height, colours.min(255))?;
        // Build an RGBA palette by combining colour index and alpha level; cap at 256 entries by alpha bucketing.
        let mut combos: Vec<[u8; 4]> = Vec::new();
        let mut map = std::collections::HashMap::<(u8, u8), u8>::new();
        let mut out_idx = Vec::with_capacity(idx.len());
        let alpha_levels: u32 = (256 / pal.len().max(1) as u32).clamp(2, 16);
        for (i, &ci) in idx.iter().enumerate() {
            let a = rgba.as_raw()[i * 4 + 3];
            let ab = ((a as u32 * (alpha_levels - 1) + 127) / 255) as u8;
            let key = (ci, ab);
            let e = *map.entry(key).or_insert_with(|| {
                let c = pal[ci as usize];
                combos.push([c[0], c[1], c[2], (ab as u32 * 255 / (alpha_levels - 1)) as u8]);
                (combos.len() - 1) as u8
            });
            out_idx.push(e);
            if combos.len() >= 256 {
                break;
            }
        }
        if out_idx.len() < idx.len() {
            // Too many colour/alpha combos: fall back to coarse alpha (2 levels).
            combos.clear();
            map.clear();
            out_idx.clear();
            for (i, &ci) in idx.iter().enumerate() {
                let a = rgba.as_raw()[i * 4 + 3];
                let ab = if a >= 128 { 1u8 } else { 0 };
                let e = *map.entry((ci, ab)).or_insert_with(|| {
                    let c = pal[ci as usize];
                    combos.push([c[0], c[1], c[2], if ab == 1 { 255 } else { 0 }]);
                    (combos.len() - 1) as u8
                });
                out_idx.push(e);
            }
        }
        (combos, out_idx)
    } else {
        let (pal, idx) = quantise_rgb(&img.rgb(), img.width, img.height, colours)?;
        (pal.iter().map(|c| [c[0], c[1], c[2], 255]).collect(), idx)
    };
    let raw = indexed_png(img.width, img.height, &palette, &indices)?;
    oxipng_optimise(&raw, img.pixels() < 2_000_000)
}

/// Write an 8-bit indexed PNG with PLTE (+tRNS when any alpha < 255).
fn indexed_png(width: u32, height: u32, palette: &[[u8; 4]], indices: &[u8]) -> Result<Vec<u8>, ImageError> {
    use std::io::Write;
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = crc32(kind);
        crc = crc32_update(crc, data);
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
    }
    fn crc32(data: &[u8]) -> u32 {
        crc32_update(0xFFFF_FFFF, data)
    }
    fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        crc
    }
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let plte: Vec<u8> = palette.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    chunk(&mut out, b"PLTE", &plte);
    if palette.iter().any(|c| c[3] < 255) {
        let trns: Vec<u8> = palette.iter().map(|c| c[3]).collect();
        chunk(&mut out, b"tRNS", &trns);
    }
    let mut raw = Vec::with_capacity((width as usize + 1) * height as usize);
    for row in indices.chunks_exact(width as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&raw).map_err(|e| ImageError::Encoder(e.to_string()))?;
    let idat = z.finish().map_err(|e| ImageError::Encoder(e.to_string()))?;
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Attach the kept ICC profile to a JPEG (APP2 segments) or leave as is.
pub fn attach_icc(bytes: Vec<u8>, icc: Option<&[u8]>, candidate: Candidate) -> Vec<u8> {
    let Some(icc) = icc else { return bytes };
    if candidate != Candidate::Jpeg || bytes.len() < 4 {
        return bytes;
    }
    // Insert after SOI (and after any JFIF APP0), one APP2 segment per 65519-byte chunk.
    let mut insert_at = 2;
    if bytes.len() > 4 && bytes[2] == 0xFF && bytes[3] == 0xE0 {
        let len = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        insert_at = 4 + len;
    }
    let chunks: Vec<&[u8]> = icc.chunks(65_519).collect();
    let mut seg = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        seg.extend_from_slice(&[0xFF, 0xE2]);
        seg.extend_from_slice(&((c.len() + 16) as u16).to_be_bytes());
        seg.extend_from_slice(b"ICC_PROFILE\0");
        seg.push((i + 1) as u8);
        seg.push(chunks.len() as u8);
        seg.extend_from_slice(c);
    }
    let mut out = Vec::with_capacity(bytes.len() + seg.len());
    out.extend_from_slice(&bytes[..insert_at]);
    out.extend_from_slice(&seg);
    out.extend_from_slice(&bytes[insert_at..]);
    out
}
