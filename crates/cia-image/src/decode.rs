//! Decode with `image` (JPEG via zune-jpeg, PNG, GIF, WebP, BMP, TIFF, QOI,
//! PNM, ICO) and JPEG XL with jxl-oxide; apply EXIF orientation; reduce to
//! 8 bits per channel (DESIGN.md 3.4.1).

use crate::metadata::{self, ExifSummary};
use crate::ImageError;
use image::{DynamicImage, GenericImageView, ImageReader};
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceFormat {
    Jpeg,
    Png,
    Webp,
    Gif,
    Bmp,
    Tiff,
    Avif,
    Heic,
    Jxl,
    Qoi,
    Pnm,
    Ico,
    Unknown,
}

impl SourceFormat {
    pub fn token(self) -> &'static str {
        match self {
            SourceFormat::Jpeg => "jpeg",
            SourceFormat::Png => "png",
            SourceFormat::Webp => "webp",
            SourceFormat::Gif => "gif",
            SourceFormat::Bmp => "bmp",
            SourceFormat::Tiff => "tiff",
            SourceFormat::Avif => "avif",
            SourceFormat::Heic => "heic",
            SourceFormat::Jxl => "jxl",
            SourceFormat::Qoi => "qoi",
            SourceFormat::Pnm => "pnm",
            SourceFormat::Ico => "ico",
            SourceFormat::Unknown => "image",
        }
    }
}

/// Sniff the container from magic bytes (extensions are never trusted).
pub fn sniff(bytes: &[u8]) -> SourceFormat {
    let b = bytes;
    if b.len() < 12 {
        return SourceFormat::Unknown;
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return SourceFormat::Jpeg;
    }
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        return SourceFormat::Png;
    }
    if &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return SourceFormat::Webp;
    }
    if b.starts_with(b"GIF8") {
        return SourceFormat::Gif;
    }
    if b.starts_with(b"BM") {
        return SourceFormat::Bmp;
    }
    if b.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || b.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return SourceFormat::Tiff;
    }
    if &b[4..8] == b"ftyp" {
        let brand = &b[8..12];
        if brand == b"avif" || brand == b"avis" {
            return SourceFormat::Avif;
        }
        if brand == b"heic" || brand == b"heix" || brand == b"hevc" || brand == b"mif1" || brand == b"heif" {
            return SourceFormat::Heic;
        }
    }
    if b.starts_with(&[0xFF, 0x0A]) || b.starts_with(&[0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20]) {
        return SourceFormat::Jxl;
    }
    if b.starts_with(b"qoif") {
        return SourceFormat::Qoi;
    }
    if b[0] == b'P' && (b'1'..=b'7').contains(&b[1]) {
        return SourceFormat::Pnm;
    }
    if b.starts_with(&[0, 0, 1, 0]) {
        return SourceFormat::Ico;
    }
    SourceFormat::Unknown
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImageInfo {
    pub format: SourceFormat,
    /// Dimensions after orientation.
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub frames: u32,
    pub orientation: u32,
    pub bits: u8,
}

/// Dimensions and format without a full decode where possible.
pub fn inspect(bytes: &[u8]) -> Result<ImageInfo, ImageError> {
    let format = sniff(bytes);
    if matches!(format, SourceFormat::Avif | SourceFormat::Heic) {
        return Err(ImageError::Unsupported(format.token().into()));
    }
    if format == SourceFormat::Jxl {
        let img = jxl_oxide::JxlImage::builder().read(Cursor::new(bytes)).map_err(|e| ImageError::Damaged(e.to_string()))?;
        return Ok(ImageInfo { format, width: img.width(), height: img.height(), has_alpha: img.image_header().metadata.alpha().is_some(), frames: 1, orientation: 1, bits: 8 });
    }
    let frames = if format == SourceFormat::Gif { count_gif_frames(bytes) } else if (format == SourceFormat::Png && is_apng(bytes)) || (format == SourceFormat::Webp && is_animated_webp(bytes)) { 2 } else { 1 };
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| ImageError::Damaged(e.to_string()))?;
    let (w, h) = reader.into_dimensions().map_err(|e| ImageError::Damaged(e.to_string()))?;
    let orientation = metadata::read_exif(bytes).map(|e| e.orientation).unwrap_or(1);
    let (w, h) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
    let has_alpha = match format {
        SourceFormat::Jpeg | SourceFormat::Bmp | SourceFormat::Pnm => false,
        SourceFormat::Png => png_has_alpha(bytes),
        _ => true, // decided for real at decode time
    };
    Ok(ImageInfo { format, width: w, height: h, has_alpha, frames, orientation, bits: if format == SourceFormat::Png && png_bit_depth(bytes) == 16 { 16 } else { 8 } })
}

fn count_gif_frames(bytes: &[u8]) -> u32 {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    opts.allow_unknown_blocks(true);
    let Ok(mut d) = opts.read_info(Cursor::new(bytes)) else { return 1 };
    let mut n = 0;
    while let Ok(Some(_)) = d.next_frame_info() {
        n += 1;
        if n > 10_000 {
            break;
        }
    }
    n.max(1)
}

fn is_apng(bytes: &[u8]) -> bool {
    bytes.windows(4).take(4096).any(|w| w == b"acTL")
}

fn is_animated_webp(bytes: &[u8]) -> bool {
    bytes.len() > 20 && &bytes[12..16] == b"VP8X" && (bytes[20] & 0x02) != 0
}

fn png_has_alpha(bytes: &[u8]) -> bool {
    bytes.len() > 26 && matches!(bytes[25], 4 | 6) || has_trns(bytes)
}

fn has_trns(bytes: &[u8]) -> bool {
    bytes.windows(4).take(8192).any(|w| w == b"tRNS")
}

fn png_bit_depth(bytes: &[u8]) -> u8 {
    if bytes.len() > 24 { bytes[24] } else { 8 }
}

/// A decoded, upright, 8-bit RGBA image plus what we need to carry forward.
#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Always RGBA8, row-major.
    pub rgba: Vec<u8>,
    pub has_alpha: bool,
    pub source: SourceFormat,
    pub source_bytes: u64,
    /// ICC profile when it is not sRGB (kept on output).
    pub icc: Option<Vec<u8>>,
    pub exif: Option<ExifSummary>,
    pub was_16_bit: bool,
}

impl DecodedImage {
    pub fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }
    pub fn rgb(&self) -> Vec<u8> {
        self.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect()
    }
    /// Composite onto white (for "Flatten transparency").
    pub fn flattened(&self) -> DecodedImage {
        let mut out = self.clone();
        for p in out.rgba.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in p.iter_mut().take(3) {
                *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
        out.has_alpha = false;
        out
    }
    pub fn from_dynamic(img: DynamicImage, source: SourceFormat, source_bytes: u64) -> Self {
        let (width, height) = img.dimensions();
        let was_16_bit = matches!(img, DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgba16(_) | DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_) | DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_));
        let has_alpha_channel = img.color().has_alpha();
        let rgba = img.into_rgba8().into_raw();
        let has_alpha = has_alpha_channel && rgba.chunks_exact(4).any(|p| p[3] < 255);
        DecodedImage { width, height, rgba, has_alpha, source, source_bytes, icc: None, exif: None, was_16_bit }
    }
}

/// Decode and normalise (3.4.1 steps 1 to 5).
pub fn decode(bytes: &[u8]) -> Result<DecodedImage, ImageError> {
    let source = sniff(bytes);
    if matches!(source, SourceFormat::Avif | SourceFormat::Heic) {
        return Err(ImageError::Unsupported(source.token().into()));
    }
    let exif = metadata::read_exif(bytes);
    let dynamic = match source {
        SourceFormat::Jxl => {
            let img = jxl_oxide::JxlImage::builder().read(Cursor::new(bytes)).map_err(|e| ImageError::Damaged(e.to_string()))?;
            let render = img.render_frame(0).map_err(|e| ImageError::Damaged(e.to_string()))?;
            let fb = render.image_all_channels();
            let (w, h, ch) = (fb.width() as u32, fb.height() as u32, fb.channels());
            let data: Vec<u8> = fb.buf().iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect();
            match ch {
                1 => DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, data).ok_or_else(|| ImageError::Damaged("jxl buffer".into()))?),
                2 => DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(w, h, data).ok_or_else(|| ImageError::Damaged("jxl buffer".into()))?),
                3 => DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, data).ok_or_else(|| ImageError::Damaged("jxl buffer".into()))?),
                _ => DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, data.chunks(ch).flat_map(|c| [c[0], c[1], c[2], c[3]]).collect()).ok_or_else(|| ImageError::Damaged("jxl buffer".into()))?),
            }
        }
        _ => {
            let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| ImageError::Damaged(e.to_string()))?;
            reader.no_limits();
            reader.decode().map_err(|e| ImageError::Damaged(e.to_string()))?
        }
    };
    let icc = read_icc(bytes, source);
    let mut img = DecodedImage::from_dynamic(dynamic, source, bytes.len() as u64);
    img.icc = icc;
    if let Some(e) = &exif {
        if e.orientation != 1 {
            apply_orientation(&mut img, e.orientation);
        }
    }
    img.exif = exif;
    Ok(img)
}

/// EXIF orientation 2..=8 to pixels; the output never carries a tag other than 1.
pub fn apply_orientation(img: &mut DecodedImage, orientation: u32) {
    let buf = image::RgbaImage::from_raw(img.width, img.height, std::mem::take(&mut img.rgba)).expect("buffer size");
    let out = match orientation {
        2 => image::imageops::flip_horizontal(&buf),
        3 => image::imageops::rotate180(&buf),
        4 => image::imageops::flip_vertical(&buf),
        5 => image::imageops::flip_horizontal(&image::imageops::rotate90(&buf)),
        6 => image::imageops::rotate90(&buf),
        7 => image::imageops::flip_horizontal(&image::imageops::rotate270(&buf)),
        8 => image::imageops::rotate270(&buf),
        _ => buf,
    };
    img.width = out.width();
    img.height = out.height();
    img.rgba = out.into_raw();
}

/// ICC profile bytes for JPEG (APP2) and PNG (iCCP); None when sRGB or absent.
fn read_icc(bytes: &[u8], source: SourceFormat) -> Option<Vec<u8>> {
    let icc = match source {
        SourceFormat::Jpeg => metadata::jpeg_icc(bytes),
        SourceFormat::Png => metadata::png_icc(bytes),
        _ => None,
    }?;
    if metadata::icc_is_srgb(&icc) {
        None
    } else {
        Some(icc)
    }
}
