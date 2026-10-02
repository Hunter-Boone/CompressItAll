//! Safe wrapper over libwebp (`libwebp-sys`) using the advanced API
//! (`WebPConfig` + `WebPPicture` + `WebPMemoryWriter`).
//!
//! Fixed encoder behaviour (DESIGN.md 3.4.3): lossy uses method 6,
//! `sns_strength` 50, `filter_strength` 60 and sharp YUV; lossless uses
//! method 6 with `exact = 0` (RGB under fully transparent pixels may be
//! altered for a smaller file); near-lossless is lossless with
//! preprocessing. Every input is validated before C runs.

#![deny(unsafe_op_in_unsafe_fn)]

use std::os::raw::c_int;

use libwebp_sys as ffi;

/// Largest width or height the WebP container allows (14 bits).
pub const MAX_DIMENSION: u32 = 16383;

/// Lossy encoder settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LossyOptions {
    /// 0.0..=100.0.
    pub quality: f32,
    /// Alpha plane quality 0..=100 (only used when the image has alpha).
    pub alpha_quality: u8,
    /// Compression effort 0..=6; the planner uses 6.
    pub method: u8,
    /// Spatial noise shaping 0..=100; the planner uses 50.
    pub sns_strength: u8,
    /// Deblocking filter strength 0..=100; the planner uses 60.
    pub filter_strength: u8,
    /// Slower, more accurate RGB to YUV conversion.
    pub sharp_yuv: bool,
}

impl LossyOptions {
    /// The planner's fixed settings at `quality` (`alpha_quality = quality`).
    pub fn planner(quality: f32) -> Self {
        Self {
            quality,
            alpha_quality: quality.clamp(0.0, 100.0).round() as u8,
            method: 6,
            sns_strength: 50,
            filter_strength: 60,
            sharp_yuv: true,
        }
    }
}

impl Default for LossyOptions {
    fn default() -> Self {
        Self::planner(75.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebpError {
    #[error("image has zero width or height")]
    ZeroSize,
    #[error("image is {width}x{height}; WebP allows at most {MAX_DIMENSION} on each side")]
    TooLarge { width: u32, height: u32 },
    #[error("pixel buffer is {actual} bytes, expected {expected} for {width}x{height} with {channels} channels")]
    BufferLength {
        expected: usize,
        actual: usize,
        width: u32,
        height: u32,
        channels: u8,
    },
    #[error("{0} is out of range")]
    InvalidOption(&'static str),
    /// libwebp rejected the configuration (`WebPValidateConfig` failed).
    #[error("libwebp rejected the encoder configuration")]
    InvalidConfig,
    /// libwebp could not initialise its structs (ABI mismatch).
    #[error("libwebp version mismatch")]
    Version,
    /// `WebPEncode` failed; the value is libwebp's `WebPEncodingError` code.
    #[error("libwebp encode failed: {0}")]
    Encode(EncodeFailure),
    #[error("not a decodable WebP bitstream")]
    Decode,
}

/// A libwebp `WebPEncodingError`, kept as the raw code plus its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeFailure(pub i32);

impl std::fmt::Display for EncodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.0 {
            1 => "out of memory",
            2 => "bitstream out of memory",
            3 => "null parameter",
            4 => "invalid configuration",
            5 => "bad dimension",
            6 => "partition 0 overflow (over 512 KiB)",
            7 => "partition overflow (over 16 MiB)",
            8 => "bad write",
            9 => "file too big (over 4 GiB)",
            10 => "user abort",
            _ => "unknown",
        };
        write!(f, "{name} (code {})", self.0)
    }
}

fn validate(pixels: &[u8], width: u32, height: u32, has_alpha: bool) -> Result<(), WebpError> {
    if width == 0 || height == 0 {
        return Err(WebpError::ZeroSize);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(WebpError::TooLarge { width, height });
    }
    let channels: u8 = if has_alpha { 4 } else { 3 };
    let expected = width as usize * height as usize * channels as usize;
    if pixels.len() != expected {
        return Err(WebpError::BufferLength {
            expected,
            actual: pixels.len(),
            width,
            height,
            channels,
        });
    }
    Ok(())
}

/// Lossy VP8 encode. `pixels` is RGBA when `has_alpha`, else RGB.
pub fn encode_lossy(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    opts: &LossyOptions,
) -> Result<Vec<u8>, WebpError> {
    validate(pixels, width, height, has_alpha)?;
    if !(0.0..=100.0).contains(&opts.quality) || opts.quality.is_nan() {
        return Err(WebpError::InvalidOption("quality"));
    }
    if opts.alpha_quality > 100 {
        return Err(WebpError::InvalidOption("alpha_quality"));
    }
    if opts.method > 6 {
        return Err(WebpError::InvalidOption("method"));
    }
    if opts.sns_strength > 100 {
        return Err(WebpError::InvalidOption("sns_strength"));
    }
    if opts.filter_strength > 100 {
        return Err(WebpError::InvalidOption("filter_strength"));
    }

    let mut config = new_config()?;
    config.lossless = 0;
    config.quality = opts.quality;
    config.alpha_quality = opts.alpha_quality as c_int;
    config.method = opts.method as c_int;
    config.sns_strength = opts.sns_strength as c_int;
    config.filter_strength = opts.filter_strength as c_int;
    config.use_sharp_yuv = opts.sharp_yuv as c_int;
    config.exact = 0;
    // SAFETY: validated input and an initialised config.
    unsafe { run_encode(&config, pixels, width, height, has_alpha, false) }
}

/// Lossless VP8L encode. `effort` 0..=100 is libwebp's lossless "quality"
/// (compression effort); method is fixed at 6 and `exact` is off.
pub fn encode_lossless(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    effort: u8,
) -> Result<Vec<u8>, WebpError> {
    validate(pixels, width, height, has_alpha)?;
    if effort > 100 {
        return Err(WebpError::InvalidOption("effort"));
    }
    let mut config = new_config()?;
    config.lossless = 1;
    config.quality = effort as f32;
    config.method = 6;
    config.exact = 0;
    // SAFETY: validated input and an initialised config.
    unsafe { run_encode(&config, pixels, width, height, has_alpha, true) }
}

/// Near-lossless VP8L encode: lossless coding after a preprocessing step that
/// trades small pixel changes for a smaller file. `near_lossless` is
/// 0..=100 where 100 is exact lossless and lower values alter more.
pub fn encode_near_lossless(
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    near_lossless: u8,
) -> Result<Vec<u8>, WebpError> {
    validate(pixels, width, height, has_alpha)?;
    if near_lossless > 100 {
        return Err(WebpError::InvalidOption("near_lossless"));
    }
    let mut config = new_config()?;
    config.lossless = 1;
    config.quality = 100.0;
    config.method = 6;
    config.near_lossless = near_lossless as c_int;
    config.exact = 0;
    // SAFETY: validated input and an initialised config.
    unsafe { run_encode(&config, pixels, width, height, has_alpha, true) }
}

/// Decode any still WebP (lossy, lossless, with or without alpha) to RGBA.
pub fn decode(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), WebpError> {
    let mut w: c_int = 0;
    let mut h: c_int = 0;
    // SAFETY: `bytes` is a valid slice for the whole call; libwebp only reads it.
    let ok = unsafe { ffi::WebPGetInfo(bytes.as_ptr(), bytes.len(), &mut w, &mut h) };
    if ok == 0 || w <= 0 || h <= 0 {
        return Err(WebpError::Decode);
    }
    let (width, height) = (w as u32, h as u32);
    let len = width as usize * height as usize * 4;
    let mut out = vec![0u8; len];
    // SAFETY: `out` is exactly width*height*4 bytes with a stride of width*4,
    // which is what WebPDecodeRGBAInto requires.
    let res = unsafe {
        ffi::WebPDecodeRGBAInto(
            bytes.as_ptr(),
            bytes.len(),
            out.as_mut_ptr(),
            len,
            (width * 4) as c_int,
        )
    };
    if res.is_null() {
        return Err(WebpError::Decode);
    }
    Ok((out, width, height))
}

fn new_config() -> Result<ffi::WebPConfig, WebpError> {
    ffi::WebPConfig::new().map_err(|()| WebpError::Version)
}

/// `WebPPicture` plus the memory writer it streams into; both are released on
/// drop whichever way the encode ends.
struct Picture {
    pic: ffi::WebPPicture,
    writer: ffi::WebPMemoryWriter,
}

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: both were initialised by libwebp in `run_encode`; the
        // library's free functions accept them in any state after init.
        unsafe {
            ffi::WebPPictureFree(&mut self.pic);
            ffi::WebPMemoryWriterClear(&mut self.writer);
        }
    }
}

/// # Safety
/// `pixels` must be `width * height * (3 or 4)` bytes and `config` initialised.
unsafe fn run_encode(
    config: &ffi::WebPConfig,
    pixels: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
    use_argb: bool,
) -> Result<Vec<u8>, WebpError> {
    // SAFETY: a NULL config is the only failure mode of WebPValidateConfig.
    if unsafe { ffi::WebPValidateConfig(config) } == 0 {
        return Err(WebpError::InvalidConfig);
    }

    let pic = ffi::WebPPicture::new().map_err(|()| WebpError::Version)?;
    // SAFETY: WebPMemoryWriter is plain data that WebPMemoryWriterInit fills.
    let mut p = Box::new(Picture {
        pic,
        writer: unsafe { std::mem::zeroed() },
    });
    unsafe {
        ffi::WebPMemoryWriterInit(&mut p.writer);
    }

    let channels = if has_alpha { 4 } else { 3 };
    let stride = (width * channels) as c_int;
    p.pic.width = width as c_int;
    p.pic.height = height as c_int;
    // Lossless (VP8L) needs ARGB; lossy wants YUV, which libwebp converts to
    // during import when use_argb is 0 (sharp YUV is applied inside WebPEncode
    // from ARGB, so import as ARGB when requested).
    p.pic.use_argb = (use_argb || config.use_sharp_yuv != 0) as c_int;
    p.pic.writer = Some(ffi::WebPMemoryWrite);
    p.pic.custom_ptr = (&mut p.writer as *mut ffi::WebPMemoryWriter).cast();

    // SAFETY: stride * height == pixels.len() (validated by the caller).
    let imported = unsafe {
        if has_alpha {
            ffi::WebPPictureImportRGBA(&mut p.pic, pixels.as_ptr(), stride)
        } else {
            ffi::WebPPictureImportRGB(&mut p.pic, pixels.as_ptr(), stride)
        }
    };
    if imported == 0 {
        return Err(WebpError::Encode(EncodeFailure(p.pic.error_code as i32)));
    }

    // SAFETY: config and picture are fully initialised; writer points at our
    // boxed WebPMemoryWriter, which outlives the call.
    if unsafe { ffi::WebPEncode(config, &mut p.pic) } == 0 {
        return Err(WebpError::Encode(EncodeFailure(p.pic.error_code as i32)));
    }

    // SAFETY: on success the writer holds `size` bytes at `mem`.
    let out = unsafe { std::slice::from_raw_parts(p.writer.mem, p.writer.size) }.to_vec();
    Ok(out)
}
