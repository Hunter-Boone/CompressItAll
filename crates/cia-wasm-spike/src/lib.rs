//! Spike gate for the wasm build: encode a generated gradient with mozjpeg
//! and libwebp from inside a wasm32-unknown-unknown module.

use cia_wasm_libc as _;
use wasm_bindgen::prelude::*;

fn gradient(width: u32, height: u32, channels: usize) -> Vec<u8> {
    let mut px = Vec::with_capacity(width as usize * height as usize * channels);
    for y in 0..height {
        for x in 0..width {
            let r = (x * 255 / width.max(2).saturating_sub(1).max(1)) as u8;
            let g = (y * 255 / height.max(2).saturating_sub(1).max(1)) as u8;
            let b = ((x ^ y) & 0xFF) as u8;
            px.extend_from_slice(&[r, g, b]);
            if channels == 4 {
                px.push((x * 255 / width.max(1)) as u8);
            }
        }
    }
    px
}

#[wasm_bindgen]
pub fn encode_jpeg(width: u32, height: u32, quality: u8) -> Result<Vec<u8>, JsError> {
    let px = gradient(width, height, 3);
    cia_mozjpeg::encode(
        &px,
        width,
        height,
        cia_mozjpeg::PixelLayout::Rgb,
        &cia_mozjpeg::JpegOptions::photo(quality),
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn encode_webp_lossless(width: u32, height: u32) -> Result<Vec<u8>, JsError> {
    let px = gradient(width, height, 4);
    cia_webp::encode_lossless(&px, width, height, true, 100)
        .map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn encode_webp_lossy(width: u32, height: u32, quality: f32) -> Result<Vec<u8>, JsError> {
    let px = gradient(width, height, 3);
    cia_webp::encode_lossy(
        &px,
        width,
        height,
        false,
        &cia_webp::LossyOptions::planner(quality),
    )
    .map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn decode_webp_dims(bytes: &[u8]) -> Result<Vec<u32>, JsError> {
    let (_, w, h) = cia_webp::decode(bytes).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(vec![w, h])
}
