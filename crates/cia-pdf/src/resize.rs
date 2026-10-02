//! Downscaling for the pixel-cap steps of the ladder (Lanczos3, via
//! fast_image_resize). Never upscales.

use fast_image_resize::images::{Image, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

/// New dimensions if the long edge exceeds `cap`, else `None`.
pub fn capped_dims(w: u32, h: u32, cap: Option<u32>) -> Option<(u32, u32)> {
    let cap = cap?;
    let long = w.max(h);
    if long <= cap || cap == 0 {
        return None;
    }
    let scale = cap as f64 / long as f64;
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    Some((nw, nh))
}

/// Resize interleaved 8-bit pixels with `channels` in {1, 3}.
pub fn downscale(
    pixels: &[u8],
    w: u32,
    h: u32,
    channels: u8,
    new_w: u32,
    new_h: u32,
) -> Result<Vec<u8>, String> {
    let pixel_type = match channels {
        1 => PixelType::U8,
        3 => PixelType::U8x3,
        n => return Err(format!("cannot resize {n}-channel image")),
    };
    let expected = w as usize * h as usize * channels as usize;
    if pixels.len() < expected {
        return Err("pixel buffer shorter than dimensions imply".into());
    }
    let src = ImageRef::new(w, h, &pixels[..expected], pixel_type).map_err(|e| e.to_string())?;
    let mut dst = Image::new(new_w, new_h, pixel_type);
    let mut resizer = Resizer::new();
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    resizer
        .resize(&src, &mut dst, &opts)
        .map_err(|e| e.to_string())?;
    Ok(dst.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_math() {
        assert_eq!(capped_dims(2000, 1500, None), None);
        assert_eq!(capped_dims(2000, 1500, Some(4000)), None);
        assert_eq!(capped_dims(2000, 1500, Some(1000)), Some((1000, 750)));
        assert_eq!(capped_dims(1500, 2000, Some(1600)), Some((1200, 1600)));
        assert_eq!(capped_dims(10, 1, Some(2)), Some((2, 1)));
    }

    #[test]
    fn downscale_keeps_flat_colour() {
        let px = vec![200u8; 64 * 48 * 3];
        let out = downscale(&px, 64, 48, 3, 16, 12).unwrap();
        assert_eq!(out.len(), 16 * 12 * 3);
        assert!(out.iter().all(|&v| v == 200));
        let g = downscale(&vec![9u8; 30 * 20], 30, 20, 1, 3, 2).unwrap();
        assert_eq!(g, vec![9u8; 6]);
    }
}
