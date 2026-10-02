//! Verification with a different decoder from the encoder (DESIGN.md 3.11, Image row).

use crate::candidates::OutputImageFormat;
use cia_core::VerificationReport;

#[derive(Debug, Clone)]
pub struct Expect {
    pub format: OutputImageFormat,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub gps_allowed: bool,
    pub hard_bytes: Option<u64>,
}

pub fn verify(bytes: &[u8], expect: &Expect) -> VerificationReport {
    let mut r = VerificationReport {
        size_ok: expect.hard_bytes.is_none_or(|h| (bytes.len() as u64) < h),
        decodes: false,
        checks: vec![],
        failures: vec![],
    };
    if !r.size_ok {
        r.failures
            .push(format!("size {} is not under the limit", bytes.len()));
    }
    let decoded: Result<(u32, u32, bool), String> = match expect.format {
        OutputImageFormat::Png => {
            // zune-png: independent of oxipng's writer and of the `png` crate.
            let mut d = zune_png::PngDecoder::new(bytes);
            match d.decode() {
                Ok(res) => {
                    let (w, h) = d.get_dimensions().unwrap_or((0, 0));
                    let alpha = d.get_colorspace().is_some_and(|c| c.has_alpha());
                    let _ = res;
                    Ok((w as u32, h as u32, alpha))
                }
                Err(e) => Err(format!("{e:?}")),
            }
        }
        OutputImageFormat::Webp => cia_webp::decode(bytes)
            .map(|(px, w, h)| (w, h, px.chunks_exact(4).any(|p| p[3] < 255)))
            .map_err(|e| e.to_string()),
        _ => image::load_from_memory(bytes)
            .map(|img| {
                use image::GenericImageView;
                let (w, h) = img.dimensions();
                let alpha =
                    img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] < 255);
                (w, h, alpha)
            })
            .map_err(|e| e.to_string()),
    };
    match decoded {
        Ok((w, h, alpha)) => {
            r.decodes = true;
            r.checks.push("decoded with a second decoder".into());
            if (w, h) != (expect.width, expect.height) {
                r.failures.push(format!(
                    "dimensions {w}×{h}, expected {}×{}",
                    expect.width, expect.height
                ));
            } else {
                r.checks.push("dimensions as planned".into());
            }
            if expect.has_alpha && !alpha && expect.format != OutputImageFormat::Jpeg {
                r.failures.push("transparency was lost".into());
            } else if expect.has_alpha {
                r.checks.push("transparency kept".into());
            }
        }
        Err(e) => r.failures.push(format!("does not decode: {e}")),
    }
    if matches!(
        expect.format,
        OutputImageFormat::Jpeg | OutputImageFormat::Png | OutputImageFormat::Webp
    ) {
        let o = crate::metadata::orientation_of(bytes);
        if o != 1 {
            r.failures
                .push(format!("EXIF orientation {o} left in output"));
        } else {
            r.checks.push("no orientation tag".into());
        }
        if !expect.gps_allowed && crate::metadata::has_gps(bytes) {
            r.failures.push("GPS data left in output".into());
        } else {
            r.checks.push(if expect.gps_allowed {
                "location kept as asked".into()
            } else {
                "no location data".into()
            });
        }
    }
    r
}
