use cia_mozjpeg::{encode, JpegError, JpegOptions, PixelLayout};
use image::{DynamicImage, ImageReader};
use std::io::Cursor;

const W: u32 = 640;
const H: u32 = 480;

/// Deterministic xorshift so the "noise" is the same on every run.
fn noise(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 24) as u8
}

fn gradient_rgb() -> Vec<u8> {
    let mut rng = 0x9E37_79B9u32;
    let mut px = Vec::with_capacity((W * H * 3) as usize);
    for y in 0..H {
        for x in 0..W {
            let n = (noise(&mut rng) >> 3) as i32 - 16;
            let r = (x * 255 / (W - 1)) as i32 + n;
            let g = (y * 255 / (H - 1)) as i32 + n;
            let b = (((x + y) * 255) / (W + H - 2)) as i32 - n;
            px.push(r.clamp(0, 255) as u8);
            px.push(g.clamp(0, 255) as u8);
            px.push(b.clamp(0, 255) as u8);
        }
    }
    px
}

fn decode(bytes: &[u8]) -> DynamicImage {
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "missing SOI marker");
    assert_eq!(
        &bytes[bytes.len() - 2..],
        &[0xFF, 0xD9],
        "missing EOI marker"
    );
    ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg)
        .decode()
        .expect("decodes as JPEG")
}

fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64)
        .sum();
    sum as f64 / a.len() as f64
}

#[test]
fn rgb_round_trip_sizes_grow_with_quality() {
    let src = gradient_rgb();
    let mut sizes = Vec::new();
    for q in [45u8, 75, 92] {
        let bytes = encode(&src, W, H, PixelLayout::Rgb, &JpegOptions::photo(q)).unwrap();
        let img = decode(&bytes);
        assert_eq!((img.width(), img.height()), (W, H));
        let rgb = img.to_rgb8();
        let err = mean_abs_diff(&src, rgb.as_raw());
        assert!(err < 8.0, "quality {q}: mean abs error {err} too high");
        sizes.push(bytes.len());
    }
    assert!(
        sizes[0] < sizes[1] && sizes[1] < sizes[2],
        "sizes not monotonic: {sizes:?}"
    );
}

#[test]
fn gray_layout() {
    let src: Vec<u8> = (0..W * H)
        .map(|i| ((i % W) * 255 / (W - 1)) as u8)
        .collect();
    let bytes = encode(&src, W, H, PixelLayout::Gray, &JpegOptions::photo(80)).unwrap();
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (W, H));
    assert_eq!(img.color(), image::ColorType::L8);
    let err = mean_abs_diff(&src, img.to_luma8().as_raw());
    assert!(err < 3.0, "gray mean abs error {err}");
}

#[test]
fn rgba_layout_drops_alpha() {
    let rgb = gradient_rgb();
    let rgba: Vec<u8> = rgb
        .chunks(3)
        .enumerate()
        .flat_map(|(i, p)| [p[0], p[1], p[2], (i % 251) as u8])
        .collect();
    let from_rgba = encode(&rgba, W, H, PixelLayout::Rgba, &JpegOptions::photo(75)).unwrap();
    let from_rgb = encode(&rgb, W, H, PixelLayout::Rgb, &JpegOptions::photo(75)).unwrap();
    assert_eq!(from_rgba, from_rgb, "alpha must not influence the output");
    let img = decode(&from_rgba);
    assert_eq!((img.width(), img.height()), (W, H));
    assert_eq!(img.color(), image::ColorType::Rgb8);
}

#[test]
fn chroma_444_and_420_both_decode() {
    let src = gradient_rgb();
    let o420 = JpegOptions {
        chroma_420: true,
        ..JpegOptions::photo(85)
    };
    let o444 = JpegOptions {
        chroma_420: false,
        ..JpegOptions::photo(85)
    };
    let b420 = encode(&src, W, H, PixelLayout::Rgb, &o420).unwrap();
    let b444 = encode(&src, W, H, PixelLayout::Rgb, &o444).unwrap();
    for b in [&b420, &b444] {
        let img = decode(b);
        assert_eq!((img.width(), img.height()), (W, H));
    }
    assert_ne!(b420, b444);
    assert!(
        b420.len() < b444.len(),
        "4:2:0 should be smaller than 4:4:4"
    );
}

#[test]
fn baseline_and_no_trellis_variants_decode() {
    let src = gradient_rgb();
    let opts = JpegOptions {
        progressive: false,
        trellis: false,
        optimize_coding: false,
        ..JpegOptions::photo(70)
    };
    let bytes = encode(&src, W, H, PixelLayout::Rgb, &opts).unwrap();
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (W, H));
}

#[test]
fn odd_dimensions_round_trip() {
    let (w, h) = (33u32, 17u32);
    let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 7 % 256) as u8).collect();
    let bytes = encode(&src, w, h, PixelLayout::Rgb, &JpegOptions::photo(90)).unwrap();
    let img = decode(&bytes);
    assert_eq!((img.width(), img.height()), (w, h));
}

#[test]
fn validation_errors_before_c() {
    let o = JpegOptions::default();
    assert_eq!(
        encode(&[], 0, 10, PixelLayout::Rgb, &o),
        Err(JpegError::ZeroSize)
    );
    assert_eq!(
        encode(&[], 10, 0, PixelLayout::Rgb, &o),
        Err(JpegError::ZeroSize)
    );
    assert!(matches!(
        encode(&[0; 11], 2, 2, PixelLayout::Rgb, &o),
        Err(JpegError::BufferLength {
            expected: 12,
            actual: 11,
            ..
        })
    ));
    assert_eq!(
        encode(
            &[0; 12],
            2,
            2,
            PixelLayout::Rgb,
            &JpegOptions { quality: 0, ..o }
        ),
        Err(JpegError::InvalidQuality(0))
    );
    assert_eq!(
        encode(
            &[0; 12],
            2,
            2,
            PixelLayout::Rgb,
            &JpegOptions { quality: 101, ..o }
        ),
        Err(JpegError::InvalidQuality(101))
    );
    assert!(matches!(
        encode(&[], 70000, 1, PixelLayout::Gray, &o),
        Err(JpegError::TooLarge { .. })
    ));
}
