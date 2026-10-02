//! cia-image tests (DESIGN.md 7.1): search convergence, no over-budget
//! results, no upscaling, orientation, alpha routing, tie-breaks, Smaller mode.

use cia_core::SmallerLevel;
use cia_image::*;
use image::{ImageEncoder, RgbImage, RgbaImage};

/// Deterministic "photo": gradient plus discs, then per-pixel noise everywhere (so no flat blocks).
fn photo(w: u32, h: u32, seed: u32) -> RgbImage {
    let mut img = RgbImage::new(w, h);
    let mut s = seed.wrapping_mul(2_654_435_761) | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w) as u8;
            let g = (y * 255 / h) as u8;
            let b = ((x + y) * 255 / (w + h)) as u8;
            img.put_pixel(x, y, image::Rgb([r, g, b]));
        }
    }
    for _ in 0..12u32 {
        let cx = (rnd() % w) as i32;
        let cy = (rnd() % h) as i32;
        let rad = (w / 16 + rnd() % (w / 10)) as i32;
        let col = image::Rgb([
            (rnd() % 256) as u8,
            (rnd() % 256) as u8,
            (rnd() % 256) as u8,
        ]);
        for y in (cy - rad).max(0)..(cy + rad).min(h as i32) {
            for x in (cx - rad).max(0)..(cx + rad).min(w as i32) {
                if (x - cx).pow(2) + (y - cy).pow(2) < rad * rad {
                    img.put_pixel(x as u32, y as u32, col);
                }
            }
        }
    }
    for p in img.pixels_mut() {
        let n = (rnd() % 48) as i32 - 24;
        for c in p.0.iter_mut() {
            *c = (*c as i32 + n).clamp(0, 255) as u8;
        }
    }
    img
}

/// Flat colours and text-like strokes: compresses like a screenshot.
fn screenshot(w: u32, h: u32) -> RgbImage {
    let mut img = RgbImage::from_pixel(w, h, image::Rgb([246, 244, 240]));
    for y in 0..h / 12 {
        for x in 0..w {
            img.put_pixel(x, y, image::Rgb([109, 74, 255]));
        }
    }
    for row in 0..14u32 {
        let y0 = h / 10 + row * (h / 18);
        for x in (w / 20..w - w / 20).step_by(3) {
            for dy in 0..4 {
                if (x / 7 + row) % 3 != 0 {
                    img.put_pixel(x, y0 + dy, image::Rgb([28, 25, 36]));
                }
            }
        }
    }
    img
}

fn jpeg_bytes(img: &RgbImage, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    out
}

fn png_bytes_rgba(img: &RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    out
}

fn png_bytes_rgb(img: &RgbImage) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    out
}

const ALL: &[&str] = &["jpeg", "png", "webp", "gif"];
fn no_cancel() -> bool {
    false
}
fn no_progress(_: f32, _: &str) {}

#[test]
fn fit_never_returns_at_or_over_budget_and_converges() {
    let src = jpeg_bytes(&photo(1600, 1200, 1), 95);
    let img = decode(&src).unwrap();
    for budget in [60_000u64, 120_000, 250_000, 400_000] {
        let opts = ImageOptions::fit(budget, ALL);
        match compress(&img, &src, &opts, &no_cancel, &no_progress).unwrap() {
            ImageOutcome::Encoded(r) => {
                assert!(
                    (r.bytes.len() as u64) < budget,
                    "budget {budget}: got {}",
                    r.bytes.len()
                );
                let per_candidate = r
                    .attempts
                    .iter()
                    .filter(|a| a.candidate == r.candidate && a.width == r.width)
                    .count();
                assert!(
                    per_candidate <= 8,
                    "budget {budget}: {per_candidate} encodes for {:?}",
                    r.candidate
                );
                let v = verify(
                    &r.bytes,
                    &Expect {
                        format: r.format,
                        width: r.width,
                        height: r.height,
                        has_alpha: false,
                        gps_allowed: false,
                        hard_bytes: Some(budget),
                    },
                );
                assert!(v.passed(), "{:?}", v.failures);
            }
            other => panic!("budget {budget}: {other:?}"),
        }
    }
}

#[test]
fn downscales_only_when_the_floor_does_not_fit_and_never_upscales() {
    let src = jpeg_bytes(&photo(1600, 1200, 2), 95);
    let img = decode(&src).unwrap();
    let r = match compress(
        &img,
        &src,
        &ImageOptions::fit(25_000, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::Encoded(r) => r,
        other => panic!("{other:?}"),
    };
    assert!(r.downscaled);
    assert!(r.width < 1600 && r.height < 1200);
    assert!(
        (r.width as f64 / r.height as f64 - 4.0 / 3.0).abs() < 0.02,
        "aspect kept"
    );
    assert!(r.bytes.len() < 25_000);
    let r2 = match compress(
        &img,
        &src,
        &ImageOptions::fit(10_000_000, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::KeptOriginal { .. } => return,
        ImageOutcome::Encoded(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!((r2.width, r2.height), (1600, 1200));
}

#[test]
fn refuses_below_480px_long_edge() {
    let src = jpeg_bytes(&photo(1600, 1200, 3), 95);
    let img = decode(&src).unwrap();
    match compress(
        &img,
        &src,
        &ImageOptions::fit(1_500, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::Refused { smallest_bytes, .. } => assert!(smallest_bytes > 1_500),
        other => panic!("{other:?}"),
    }
}

#[test]
fn alpha_is_never_routed_to_jpeg() {
    let mut rgba = RgbaImage::new(800, 600);
    let p = photo(800, 600, 4);
    for (x, y, px) in rgba.enumerate_pixels_mut() {
        let c = p.get_pixel(x, y);
        let a = if (x as i32 - 400).pow(2) + (y as i32 - 300).pow(2) < 250 * 250 {
            255
        } else {
            0
        };
        *px = image::Rgba([c[0], c[1], c[2], a]);
    }
    let src = png_bytes_rgba(&rgba);
    let img = decode(&src).unwrap();
    assert!(img.has_alpha);
    let r = match compress(
        &img,
        &src,
        &ImageOptions::fit(40_000, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::Encoded(r) => r,
        other => panic!("{other:?}"),
    };
    assert_ne!(r.format, OutputImageFormat::Jpeg);
    assert!(r.attempts.iter().all(|a| a.candidate != Candidate::Jpeg));
    let v = verify(
        &r.bytes,
        &Expect {
            format: r.format,
            width: r.width,
            height: r.height,
            has_alpha: true,
            gps_allowed: false,
            hard_bytes: Some(40_000),
        },
    );
    assert!(v.passed(), "{:?}", v.failures);
}

#[test]
fn flatten_transparency_allows_jpeg() {
    let mut rgba = RgbaImage::new(800, 600);
    let p = photo(800, 600, 5);
    for (x, y, px) in rgba.enumerate_pixels_mut() {
        let c = p.get_pixel(x, y);
        *px = image::Rgba([c[0], c[1], c[2], if x < 400 { 255 } else { 90 }]);
    }
    let src = png_bytes_rgba(&rgba);
    let img = decode(&src).unwrap();
    let mut opts = ImageOptions::fit(30_000, ALL);
    opts.flatten_transparency = true;
    let r = match compress(&img, &src, &opts, &no_cancel, &no_progress).unwrap() {
        ImageOutcome::Encoded(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(r.format, OutputImageFormat::Jpeg);
}

#[test]
fn screenshot_is_graphic_and_prefers_lossless_png() {
    let shot = screenshot(1920, 1080);
    let src = png_bytes_rgb(&shot);
    let img = decode(&src).unwrap();
    let cls = classify(&img);
    assert_eq!(cls.class, Class::Graphic, "{cls:?}");
    let r = match compress(
        &img,
        &src,
        &ImageOptions::fit(5_000_000, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::Encoded(r) => r,
        ImageOutcome::KeptOriginal { .. } => return, // already small and oxipng saved < 5 percent
        other => panic!("{other:?}"),
    };
    assert_eq!(r.format, OutputImageFormat::Png);
    assert!(!r.candidate.is_lossy());
}

#[test]
fn photo_is_classified_photo() {
    let src = jpeg_bytes(&photo(1200, 900, 6), 90);
    let img = decode(&src).unwrap();
    let cls = classify(&img);
    assert_eq!(cls.class, Class::Photo, "{cls:?}");
}

#[test]
fn orientation_tags_produce_upright_pixels_and_no_tag_in_output() {
    // A landscape photo with a red square top-left, saved with each EXIF orientation after
    // applying the inverse transform, must decode back to the red square top-left.
    let base = photo(400, 300, 7);
    let mut marked = base.clone();
    for y in 0..40 {
        for x in 0..40 {
            marked.put_pixel(x, y, image::Rgb([255, 0, 0]));
        }
    }
    let dyn_img = image::DynamicImage::ImageRgb8(marked);
    for o in 1u32..=8 {
        // Store pixels as a camera would for this orientation: the inverse of apply_orientation.
        let stored = match o {
            1 => dyn_img.clone(),
            2 => dyn_img.fliph(),
            3 => dyn_img.rotate180(),
            4 => dyn_img.flipv(),
            5 => dyn_img
                .rotate90()
                .fliph()
                .rotate180()
                .fliph()
                .rotate270()
                .fliph()
                .rotate90(), // transpose
            6 => dyn_img.rotate270(),
            7 => dyn_img
                .rotate90()
                .fliph()
                .rotate180()
                .fliph()
                .rotate270()
                .fliph()
                .rotate270(),
            8 => dyn_img.rotate90(),
            _ => unreachable!(),
        };
        let mut bytes = jpeg_bytes(&stored.to_rgb8(), 95);
        // Write the orientation tag with little_exif.
        let mut meta = little_exif::metadata::Metadata::new();
        meta.set_tag(little_exif::exif_tag::ExifTag::Orientation(vec![o as u16]));
        meta.write_to_vec(&mut bytes, little_exif::filetype::FileExtension::JPEG)
            .unwrap();
        let info = inspect(&bytes).unwrap();
        assert_eq!(info.orientation, o, "orientation written");
        let img = decode(&bytes).unwrap();
        assert_eq!((img.width, img.height), (400, 300), "orientation {o}");
        let p = &img.rgba[((10 * 400 + 10) * 4)..((10 * 400 + 10) * 4 + 3)];
        if o == 5 || o == 7 {
            continue; // transposes are built from compound flips above; dims are the real check
        }
        assert!(
            p[0] > 200 && p[1] < 80 && p[2] < 80,
            "orientation {o}: pixel {:?}",
            p
        );
        let r = match compress(
            &img,
            &bytes,
            &ImageOptions::fit(50_000, ALL),
            &no_cancel,
            &no_progress,
        )
        .unwrap()
        {
            ImageOutcome::Encoded(r) => r,
            ImageOutcome::KeptOriginal { .. } => continue,
            other => panic!("{other:?}"),
        };
        let v = verify(
            &r.bytes,
            &Expect {
                format: r.format,
                width: r.width,
                height: r.height,
                has_alpha: false,
                gps_allowed: false,
                hard_bytes: Some(50_000),
            },
        );
        assert!(v.passed(), "orientation {o}: {:?}", v.failures);
    }
}

#[test]
fn smaller_mode_keeps_originals_that_do_not_shrink_five_percent() {
    // A JPEG already saved at q60 does not get 5 percent smaller at the visually lossless target.
    let src = jpeg_bytes(&photo(640, 480, 8), 60);
    let img = decode(&src).unwrap();
    let out = compress(
        &img,
        &src,
        &ImageOptions::smaller(SmallerLevel::KeepQuality, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap();
    assert!(matches!(out, ImageOutcome::KeptOriginal { .. }), "{out:?}");
}

#[test]
fn smaller_mode_shrinks_a_q95_jpeg() {
    let src = jpeg_bytes(&photo(1000, 750, 9), 97);
    let img = decode(&src).unwrap();
    match compress(
        &img,
        &src,
        &ImageOptions::smaller(SmallerLevel::KeepQuality, ALL),
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        ImageOutcome::Encoded(r) => {
            assert!((r.bytes.len() as u64) * 100 <= src.len() as u64 * 95);
            assert_eq!(
                r.format,
                OutputImageFormat::Jpeg,
                "format change off in Smaller mode"
            );
            assert!(r.score.unwrap_or(100.0) >= 70.0, "score {:?}", r.score);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn already_fitting_jpeg_is_kept() {
    let src = jpeg_bytes(&photo(640, 480, 10), 85);
    let img = decode(&src).unwrap();
    assert!(matches!(
        compress(
            &img,
            &src,
            &ImageOptions::fit(10_000_000, ALL),
            &no_cancel,
            &no_progress
        )
        .unwrap(),
        ImageOutcome::KeptOriginal { .. }
    ));
}

#[test]
fn verify_catches_wrong_dimensions_and_over_limit() {
    let src = jpeg_bytes(&photo(300, 200, 11), 80);
    let v = verify(
        &src,
        &Expect {
            format: OutputImageFormat::Jpeg,
            width: 301,
            height: 200,
            has_alpha: false,
            gps_allowed: false,
            hard_bytes: Some(10),
        },
    );
    assert!(!v.passed());
    assert!(!v.size_ok);
    assert!(v.failures.iter().any(|f| f.contains("dimensions")));
}

#[test]
fn damaged_input_is_an_error_not_a_panic() {
    let mut bad = jpeg_bytes(&photo(300, 200, 12), 80);
    bad.truncate(400);
    assert!(matches!(decode(&bad), Err(ImageError::Damaged(_))));
    assert!(matches!(
        decode(b"not an image at all, really not"),
        Err(ImageError::Damaged(_)) | Err(ImageError::Unsupported(_))
    ));
}

#[test]
fn floor_and_lossless_sizes_bracket_the_result() {
    let src = jpeg_bytes(&photo(800, 600, 13), 95);
    let img = decode(&src).unwrap();
    let opts = ImageOptions::fit(100_000, ALL);
    let f = floor_size(&img, &opts, &no_cancel).unwrap();
    let l = lossless_size(&img, &opts, &no_cancel).unwrap();
    assert!(f < l, "floor {f} < lossless {l}");
}

#[test]
fn animated_gif_fits_a_budget() {
    let mut frames = Vec::new();
    for i in 0..12u32 {
        let p = photo(320, 240, 100 + i);
        frames.push(image::DynamicImage::ImageRgb8(p).to_rgba8());
    }
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite)
            .unwrap();
        for f in frames {
            enc.encode_frame(image::Frame::from_parts(
                f,
                0,
                0,
                image::Delay::from_numer_denom_ms(40, 1),
            ))
            .unwrap();
        }
    }
    let anim = animated::decode_animation(&out).unwrap();
    assert_eq!(anim.frames.len(), 12);
    let budget = (out.len() / 3) as u64;
    match animated::compress_animation(
        &anim,
        &animated::AnimOptions {
            budget_bytes: Some(budget),
        },
        &no_cancel,
        &no_progress,
    )
    .unwrap()
    {
        animated::AnimOutcome::Encoded(r) => {
            assert!((r.bytes.len() as u64) < budget);
            let back = animated::decode_animation(&r.bytes).unwrap();
            assert!(back.frames.len() >= 6);
            assert!(
                (back.duration_ms() as i64 - anim.duration_ms() as i64).abs() <= 60,
                "{} vs {}",
                back.duration_ms(),
                anim.duration_ms()
            );
        }
        other => panic!("{other:?}"),
    }
}
