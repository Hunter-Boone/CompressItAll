//! Safe wrapper over mozjpeg (`mozjpeg-sys`) that encodes an 8-bit pixel
//! buffer to a JPEG in memory.
//!
//! Fixed encoder behaviour (DESIGN.md 3.4.3): `JCP_MAX_COMPRESSION` profile,
//! optimized progressive scans, trellis quantisation and optimized Huffman
//! tables, each switchable through [`JpegOptions`], plus 4:2:0 or 4:4:4
//! chroma subsampling per call.
//!
//! Error handling: libjpeg reports fatal errors through `error_exit`, which
//! must not return. The default implementation calls `exit()`; the usual
//! replacement is `longjmp`, which is not sound across Rust frames. Instead our
//! `extern "C-unwind"` callback panics. On native the C code is compiled with
//! `-fexceptions` (mozjpeg-sys `unwinding` feature) so the panic unwinds
//! through libjpeg and is caught with `catch_unwind` at the FFI boundary,
//! turning into [`JpegError::Encoder`]. On wasm32 (`panic = "abort"`) the
//! panic traps the instance; the host treats a trap as a failed attempt. Every
//! input is validated before C runs, so traps do not happen in practice.
//!
//! This software is based in part on the work of the Independent JPEG Group.

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::CStr;
use std::mem;
use std::os::raw::{c_int, c_ulong, c_void};
use std::ptr;

use mozjpeg_sys as ffi;

/// Largest width or height libjpeg accepts (`JPEG_MAX_DIMENSION`).
pub const MAX_DIMENSION: u32 = 65500;

/// Encoder settings. Quality is the only value the planner searches; the rest
/// are fixed per candidate (DESIGN.md 3.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegOptions {
    /// 1..=100 on the IJG scale.
    pub quality: u8,
    /// Progressive scans (with mozjpeg's scan optimisation) instead of baseline.
    pub progressive: bool,
    /// 4:2:0 chroma subsampling; `false` keeps full 4:4:4 chroma. Ignored for Gray.
    pub chroma_420: bool,
    /// Optimized Huffman tables (two-pass).
    pub optimize_coding: bool,
    /// Trellis quantisation of AC and DC coefficients.
    pub trellis: bool,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self {
            quality: 75,
            progressive: true,
            chroma_420: true,
            optimize_coding: true,
            trellis: true,
        }
    }
}

impl JpegOptions {
    /// The planner's fixed settings for a photo at `quality`: everything on,
    /// 4:2:0 below quality 90 and 4:4:4 from 90 up.
    pub fn photo(quality: u8) -> Self {
        Self {
            quality,
            chroma_420: quality < 90,
            ..Self::default()
        }
    }

    /// The planner's fixed settings for a graphic: always 4:4:4 so text stays sharp.
    pub fn graphic(quality: u8) -> Self {
        Self {
            quality,
            chroma_420: false,
            ..Self::default()
        }
    }
}

/// Memory layout of the input pixel buffer, 8 bits per channel, rows packed
/// with no padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    /// 3 bytes per pixel.
    Rgb,
    /// 4 bytes per pixel; the alpha channel is dropped (JPEG has no alpha).
    Rgba,
    /// 1 byte per pixel, written as a single-component grayscale JPEG.
    Gray,
}

impl PixelLayout {
    /// Bytes per pixel in the input buffer.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            PixelLayout::Rgb => 3,
            PixelLayout::Rgba => 4,
            PixelLayout::Gray => 1,
        }
    }

    fn color_space(self) -> ffi::J_COLOR_SPACE {
        match self {
            PixelLayout::Rgb => ffi::J_COLOR_SPACE::JCS_EXT_RGB,
            PixelLayout::Rgba => ffi::J_COLOR_SPACE::JCS_EXT_RGBA,
            PixelLayout::Gray => ffi::J_COLOR_SPACE::JCS_GRAYSCALE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JpegError {
    #[error("image has zero width or height")]
    ZeroSize,
    #[error("image is {width}x{height}; JPEG allows at most {MAX_DIMENSION} on each side")]
    TooLarge { width: u32, height: u32 },
    #[error("pixel buffer is {actual} bytes, expected {expected} for {width}x{height} {layout:?}")]
    BufferLength {
        expected: usize,
        actual: usize,
        width: u32,
        height: u32,
        layout: PixelLayout,
    },
    #[error("quality must be 1..=100, got {0}")]
    InvalidQuality(u8),
    /// libjpeg reported a fatal error (its message is included).
    #[error("mozjpeg: {0}")]
    Encoder(String),
}

/// Encode `pixels` (`width * height * bpp` bytes, row-major, top to bottom)
/// to a JPEG file in memory.
pub fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    layout: PixelLayout,
    opts: &JpegOptions,
) -> Result<Vec<u8>, JpegError> {
    if width == 0 || height == 0 {
        return Err(JpegError::ZeroSize);
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(JpegError::TooLarge { width, height });
    }
    if !(1..=100).contains(&opts.quality) {
        return Err(JpegError::InvalidQuality(opts.quality));
    }
    let expected = width as usize * height as usize * layout.bytes_per_pixel();
    if pixels.len() != expected {
        return Err(JpegError::BufferLength {
            expected,
            actual: pixels.len(),
            width,
            height,
            layout,
        });
    }

    guarded(|| {
        // SAFETY: inputs were validated above; `encode_raw` upholds libjpeg's
        // call sequence and owns every resource through `Compressor`.
        unsafe { encode_raw(pixels, width, height, layout, opts) }
    })
}

/// Runs the encoder, converting a panic raised by `error_exit` into
/// `JpegError::Encoder` on targets that can unwind.
#[cfg(not(target_arch = "wasm32"))]
fn guarded(f: impl FnOnce() -> Vec<u8>) -> Result<Vec<u8>, JpegError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(bytes) => Ok(bytes),
        Err(payload) => Err(JpegError::Encoder(panic_message(&*payload))),
    }
}

/// With `panic = "abort"` there is nothing to catch; a libjpeg error traps.
#[cfg(target_arch = "wasm32")]
fn guarded(f: impl FnOnce() -> Vec<u8>) -> Result<Vec<u8>, JpegError> {
    Ok(f())
}

#[cfg(not(target_arch = "wasm32"))]
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "unknown error".to_string()
    }
}

// The `free()` that matches libjpeg's `malloc()`: libc on native, the
// `cia-wasm-libc` shim on wasm32. Declared here because the `libc` crate has
// no `free` on wasm32-unknown-unknown.
extern "C" {
    fn free(p: *mut c_void);
}

/// Owns the libjpeg compress object, its error manager and the memory
/// destination buffer. Boxed so the self-referential `cinfo.err` pointer
/// never moves. `Drop` runs on the normal path and when `error_exit` unwinds.
struct Compressor {
    cinfo: ffi::jpeg_compress_struct,
    err: ffi::jpeg_error_mgr,
    out_buf: *mut u8,
    out_size: c_ulong,
}

impl Compressor {
    unsafe fn new() -> Box<Self> {
        // SAFETY: both structs are plain C data that libjpeg initialises;
        // all-zero is a valid "not yet created" state for them.
        let mut c = Box::new(Self {
            cinfo: unsafe { mem::zeroed() },
            err: unsafe { mem::zeroed() },
            out_buf: ptr::null_mut(),
            out_size: 0,
        });
        unsafe {
            ffi::jpeg_std_error(&mut c.err);
        }
        c.err.error_exit = Some(error_exit);
        c.err.output_message = Some(output_message);
        c.cinfo.common.err = &mut c.err;
        unsafe {
            ffi::jpeg_create_compress(&mut c.cinfo);
        }
        c
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        // SAFETY: cinfo was created in `new`; destroy is valid in any state
        // after creation. The output buffer was malloc'd by jpeg_mem_dest.
        unsafe {
            ffi::jpeg_destroy_compress(&mut self.cinfo);
            if !self.out_buf.is_null() {
                free(self.out_buf as *mut c_void);
            }
        }
    }
}

/// Replacement for libjpeg's `error_exit`: formats the message and panics
/// instead of calling `exit()`. See the crate docs for why this is sound.
unsafe extern "C-unwind" fn error_exit(cinfo: &mut ffi::jpeg_common_struct) {
    let mut buffer = [0u8; 80];
    // SAFETY: `err` is our `jpeg_error_mgr`, installed in `Compressor::new`,
    // and `format_message` is libjpeg's default (we never replace it).
    let message = unsafe {
        if let Some(format) = (*cinfo.err).format_message {
            format(cinfo, &buffer);
            CStr::from_ptr(buffer.as_ptr().cast())
                .to_string_lossy()
                .into_owned()
        } else {
            format!("error code {}", (*cinfo.err).msg_code)
        }
    };
    let _ = &mut buffer;
    std::panic::panic_any(message);
}

/// Warnings go nowhere; the default writes to stderr.
unsafe extern "C-unwind" fn output_message(_cinfo: &mut ffi::jpeg_common_struct) {}

fn bool_param(v: bool) -> ffi::boolean {
    if v {
        1
    } else {
        0
    }
}

/// # Safety
/// Caller validated dimensions, quality and buffer length.
unsafe fn encode_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    layout: PixelLayout,
    opts: &JpegOptions,
) -> Vec<u8> {
    let mut c = unsafe { Compressor::new() };
    let Compressor {
        cinfo,
        out_buf,
        out_size,
        ..
    } = &mut *c;

    unsafe {
        ffi::jpeg_mem_dest(cinfo, out_buf, out_size);

        cinfo.image_width = width;
        cinfo.image_height = height;
        cinfo.input_components = layout.bytes_per_pixel() as c_int;
        cinfo.in_color_space = layout.color_space();

        // The profile must be set before jpeg_set_defaults, which derives the
        // mozjpeg extension defaults (progressive scans, trellis) from it.
        ffi::jpeg_c_set_int_param(
            cinfo,
            ffi::J_INT_PARAM::JINT_COMPRESS_PROFILE,
            ffi::JINT_COMPRESS_PROFILE_VALUE::JCP_MAX_COMPRESSION as c_int,
        );
        ffi::jpeg_set_defaults(cinfo);
        ffi::jpeg_set_quality(cinfo, opts.quality as c_int, bool_param(true));

        if opts.progressive {
            ffi::jpeg_c_set_bool_param(
                cinfo,
                ffi::J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS,
                bool_param(true),
            );
            ffi::jpeg_simple_progression(cinfo);
        } else {
            ffi::jpeg_c_set_bool_param(
                cinfo,
                ffi::J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS,
                bool_param(false),
            );
            cinfo.scan_info = ptr::null();
            cinfo.num_scans = 0;
        }

        ffi::jpeg_c_set_bool_param(
            cinfo,
            ffi::J_BOOLEAN_PARAM::JBOOLEAN_TRELLIS_QUANT,
            bool_param(opts.trellis),
        );
        ffi::jpeg_c_set_bool_param(
            cinfo,
            ffi::J_BOOLEAN_PARAM::JBOOLEAN_TRELLIS_QUANT_DC,
            bool_param(opts.trellis),
        );
        cinfo.optimize_coding = bool_param(opts.optimize_coding);

        if layout != PixelLayout::Gray {
            // Component 0 is luma; chroma components default to 1x1, so luma
            // 2x2 gives 4:2:0 and 1x1 gives 4:4:4.
            let samp = if opts.chroma_420 { 2 } else { 1 };
            let luma = &mut *cinfo.comp_info;
            luma.h_samp_factor = samp;
            luma.v_samp_factor = samp;
        }

        ffi::jpeg_start_compress(cinfo, bool_param(true));

        let stride = width as usize * layout.bytes_per_pixel();
        while cinfo.next_scanline < cinfo.image_height {
            let row = pixels.as_ptr().add(cinfo.next_scanline as usize * stride);
            let rows: [ffi::JSAMPROW; 1] = [row];
            ffi::jpeg_write_scanlines(cinfo, rows.as_ptr(), 1);
        }

        ffi::jpeg_finish_compress(cinfo);

        std::slice::from_raw_parts(*out_buf, *out_size as usize).to_vec()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// Drive libjpeg into a real `ERREXIT` (writing scanlines before
    /// `jpeg_start_compress`) and check the panic is caught and reported.
    #[test]
    fn libjpeg_error_becomes_jpeg_error() {
        let result = guarded(|| unsafe {
            let mut c = Compressor::new();
            let row = [0u8; 3];
            let rows: [ffi::JSAMPROW; 1] = [row.as_ptr()];
            ffi::jpeg_write_scanlines(&mut c.cinfo, rows.as_ptr(), 1);
            Vec::new()
        });
        match result {
            Err(JpegError::Encoder(msg)) => {
                assert!(msg.contains("Improper call"), "message was {msg:?}")
            }
            other => panic!("expected Encoder error, got {other:?}"),
        }
    }
}
