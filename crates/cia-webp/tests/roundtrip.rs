use cia_webp::{
    decode, encode_lossless, encode_lossy, encode_near_lossless, LossyOptions, WebpError,
};
use image::ImageReader;
use std::io::Cursor;

const W: u32 = 320;
const H: u32 = 240;

fn noise(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 24) as u8
}

fn gradient(channels: usize) -> Vec<u8> {
    let mut rng = 0x1234_5678u32;
    let mut px = Vec::with_capacity((W * H) as usize * channels);
    for y in 0..H {
        for x in 0..W {
            let n = (noise(&mut rng) >> 3) as i32 - 16;
            let r = ((x * 255 / (W - 1)) as i32 + n).clamp(0, 255) as u8;
            let g = ((y * 255 / (H - 1)) as i32 + n).clamp(0, 255) as u8;
            let b = ((((x + y) * 255) / (W + H - 2)) as i32 - n).clamp(0, 255) as u8;
            px.extend_from_slice(&[r, g, b]);
            if channels == 4 {
                // Varied alpha: opaque band, gradient band, fully transparent corner.
                let a = if y < H / 3 {
                    255
                } else if x < W / 4 && y > 2 * H / 3 {
                    0
                } else {
                    (x * 255 / (W - 1)) as u8
                };
                px.push(a);
            }
        }
    }
    px
}

fn check_container(bytes: &[u8]) {
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");
    let riff_len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    assert_eq!(riff_len + 8, bytes.len(), "RIFF length field mismatch");
}

fn decode_with_image_crate(bytes: &[u8]) -> (u32, u32) {
    let img = ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::WebP)
        .decode()
        .expect("image crate decodes it");
    (img.width(), img.height())
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
fn lossy_rgb_sizes_grow_with_quality() {
    let src = gradient(3);
    let mut sizes = Vec::new();
    for q in [50.0f32, 80.0, 92.0] {
        let bytes = encode_lossy(&src, W, H, false, &LossyOptions::planner(q)).unwrap();
        check_container(&bytes);
        assert_eq!(decode_with_image_crate(&bytes), (W, H));
        let (rgba, w, h) = decode(&bytes).unwrap();
        assert_eq!((w, h), (W, H));
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let err = mean_abs_diff(&src, &rgb);
        assert!(err < 8.0, "quality {q}: mean abs error {err}");
        sizes.push(bytes.len());
    }
    assert!(
        sizes[0] < sizes[1] && sizes[1] < sizes[2],
        "sizes not monotonic: {sizes:?}"
    );
}

#[test]
fn lossy_rgba_keeps_alpha() {
    let src = gradient(4);
    let bytes = encode_lossy(&src, W, H, true, &LossyOptions::planner(80.0)).unwrap();
    check_container(&bytes);
    let (rgba, w, h) = decode(&bytes).unwrap();
    assert_eq!((w, h), (W, H));
    let src_a: Vec<u8> = src.iter().skip(3).step_by(4).copied().collect();
    let out_a: Vec<u8> = rgba.iter().skip(3).step_by(4).copied().collect();
    let err = mean_abs_diff(&src_a, &out_a);
    assert!(err < 4.0, "alpha mean abs error {err}");
}

#[test]
fn lossless_rgba_is_pixel_exact() {
    let src = gradient(4);
    let bytes = encode_lossless(&src, W, H, true, 100).unwrap();
    check_container(&bytes);
    assert_eq!(decode_with_image_crate(&bytes), (W, H));
    let (rgba, w, h) = decode(&bytes).unwrap();
    assert_eq!((w, h), (W, H));
    // exact = 0 may alter RGB under alpha == 0; compare everything else exactly.
    for (i, (s, o)) in src.chunks(4).zip(rgba.chunks(4)).enumerate() {
        assert_eq!(s[3], o[3], "alpha differs at pixel {i}");
        if s[3] != 0 {
            assert_eq!(s, o, "pixel {i} differs");
        }
    }
}

#[test]
fn lossless_rgb_is_pixel_exact_and_effort_matters() {
    let src = gradient(3);
    let fast = encode_lossless(&src, W, H, false, 0).unwrap();
    let slow = encode_lossless(&src, W, H, false, 100).unwrap();
    for b in [&fast, &slow] {
        check_container(b);
        let (rgba, w, h) = decode(b).unwrap();
        assert_eq!((w, h), (W, H));
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        assert_eq!(rgb, src);
    }
    assert!(
        slow.len() <= fast.len(),
        "effort 100 ({}) larger than effort 0 ({})",
        slow.len(),
        fast.len()
    );
}

#[test]
fn near_lossless_decodes_and_is_smaller_than_lossless() {
    let src = gradient(3);
    let exact = encode_lossless(&src, W, H, false, 100).unwrap();
    let near = encode_near_lossless(&src, W, H, false, 60).unwrap();
    check_container(&near);
    let (rgba, w, h) = decode(&near).unwrap();
    assert_eq!((w, h), (W, H));
    let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    assert!(mean_abs_diff(&src, &rgb) < 4.0);
    assert!(
        near.len() < exact.len(),
        "near {} vs exact {}",
        near.len(),
        exact.len()
    );
}

#[test]
fn validation_errors_before_c() {
    let o = LossyOptions::default();
    assert_eq!(encode_lossy(&[], 0, 4, false, &o), Err(WebpError::ZeroSize));
    assert!(matches!(
        encode_lossy(&[0; 47], 4, 4, false, &o),
        Err(WebpError::BufferLength {
            expected: 48,
            actual: 47,
            ..
        })
    ));
    assert!(matches!(
        encode_lossy(&[0; 48], 4, 4, true, &o),
        Err(WebpError::BufferLength {
            expected: 64,
            actual: 48,
            ..
        })
    ));
    assert!(matches!(
        encode_lossless(&[0; 10], 4, 4, false, 100),
        Err(WebpError::BufferLength { .. })
    ));
    assert_eq!(
        encode_lossless(&[0; 48], 4, 4, false, 101),
        Err(WebpError::InvalidOption("effort"))
    );
    assert!(matches!(
        encode_near_lossless(&[0; 10], 4, 4, false, 60),
        Err(WebpError::BufferLength { .. })
    ));
    assert_eq!(
        encode_lossy(
            &[0; 48],
            4,
            4,
            false,
            &LossyOptions {
                quality: 101.0,
                ..o
            }
        ),
        Err(WebpError::InvalidOption("quality"))
    );
    assert!(matches!(
        encode_lossy(&[], 20000, 1, false, &o),
        Err(WebpError::TooLarge { .. })
    ));
    assert_eq!(decode(b"not a webp file at all"), Err(WebpError::Decode));
    assert_eq!(decode(&[]), Err(WebpError::Decode));
}
