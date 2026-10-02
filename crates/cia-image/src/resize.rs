//! Downscaling with Lanczos3 (DESIGN.md 3.4.6): linear light for photos, sRGB for graphics.

use crate::decode::DecodedImage;
use fast_image_resize as fr;
use image::{RgbImage, RgbaImage};

/// Scale so the long edge becomes `long_edge`, keeping aspect, dims >= 1.
pub fn dims_for_long_edge(w: u32, h: u32, long_edge: u32) -> (u32, u32) {
    let (lw, lh) = (w as f64, h as u64 as f64);
    let scale = long_edge as f64 / lw.max(lh);
    (((lw * scale).round() as u32).max(1), ((lh * scale).round() as u32).max(1))
}

pub fn downscale(img: &DecodedImage, new_w: u32, new_h: u32, linear_light: bool) -> DecodedImage {
    let src = RgbaImage::from_raw(img.width, img.height, img.rgba.clone()).expect("buffer");
    let src_img = image::DynamicImage::ImageRgba8(src);
    let mut dst = fr::images::Image::new(new_w, new_h, fr::PixelType::U8x4);
    let mut resizer = fr::Resizer::new();
    let alg = fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3);
    let opts = fr::ResizeOptions::new().resize_alg(alg);
    if linear_light {
        let mut src_view = fr::images::Image::from_vec_u8(img.width, img.height, img.rgba.clone(), fr::PixelType::U8x4).expect("buffer");
        let mapper = fr::create_srgb_mapper();
        let _ = mapper.forward_map_inplace(&mut src_view);
        let _ = resizer.resize(&src_view, &mut dst, &opts);
        let _ = mapper.backward_map_inplace(&mut dst);
    } else {
        let _ = resizer.resize(&src_img, &mut dst, &opts);
    }
    DecodedImage { width: new_w, height: new_h, rgba: dst.into_vec(), has_alpha: img.has_alpha, source: img.source, source_bytes: img.source_bytes, icc: img.icc.clone(), exif: img.exif.clone(), was_16_bit: img.was_16_bit }
}

/// RGB thumbnail by nearest-neighbour sampling (no averaging, so photographic
/// noise survives and the flat-block test still tells photos from graphics).
pub fn thumbnail_rgb(img: &DecodedImage, max_edge: u32) -> RgbImage {
    let (w, h) = if img.long_edge() <= max_edge { (img.width, img.height) } else { dims_for_long_edge(img.width, img.height, max_edge) };
    let src = fr::images::Image::from_vec_u8(img.width, img.height, img.rgba.clone(), fr::PixelType::U8x4).expect("buffer");
    let mut dst = fr::images::Image::new(w, h, fr::PixelType::U8x4);
    let mut resizer = fr::Resizer::new();
    let _ = resizer.resize(&src, &mut dst, &fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Nearest));
    let rgb: Vec<u8> = dst.buffer().chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    RgbImage::from_raw(w, h, rgb).expect("buffer")
}
