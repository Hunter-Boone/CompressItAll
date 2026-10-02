//! SSIMULACRA2 between source and result, both downscaled to at most 1 MP
//! with the same filter (DESIGN.md 3.4.5).

use crate::decode::DecodedImage;
use ssimulacra2::{compute_frame_ssimulacra2, ColorPrimaries, Rgb, TransferCharacteristic};

const MAX_PIXELS: u64 = 1_000_000;

fn to_rgb_f32(img: &DecodedImage) -> Rgb {
    let data: Vec<[f32; 3]> = img.rgba.chunks_exact(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
    Rgb::new(data, img.width as usize, img.height as usize, TransferCharacteristic::SRGB, ColorPrimaries::BT709).expect("valid rgb")
}

fn capped(img: &DecodedImage) -> DecodedImage {
    if img.pixels() <= MAX_PIXELS {
        return img.clone();
    }
    let scale = (MAX_PIXELS as f64 / img.pixels() as f64).sqrt();
    let w = ((img.width as f64 * scale) as u32).max(8);
    let h = ((img.height as f64 * scale) as u32).max(8);
    crate::resize::downscale(img, w, h, false)
}

/// Returns None when the images differ in size after capping (should not happen) or the metric fails.
pub fn ssimulacra2(source: &DecodedImage, result: &DecodedImage) -> Option<f32> {
    let a = capped(source);
    let b = if result.width == source.width && result.height == source.height { capped(result) } else { crate::resize::downscale(result, a.width, a.height, false) };
    if a.width != b.width || a.height != b.height || a.width < 8 || a.height < 8 {
        return None;
    }
    compute_frame_ssimulacra2(to_rgb_f32(&a), to_rgb_f32(&b)).ok().map(|v| v as f32)
}
