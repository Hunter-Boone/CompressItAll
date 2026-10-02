//! Round trips over the fixtures plus deliberately broken inputs and outputs.

use cia_pdf::{
    inspect, optimise, verify, JpegEncoder, PdfError, PdfOptions, PdfOutcome, SmallerLevel,
};
use lopdf::{dictionary, Document, Object, Stream};

const SCAN: &[u8] = include_bytes!("fixtures/scan_photos.pdf");
const VECTOR: &[u8] = include_bytes!("fixtures/vector_text.pdf");
const ENCRYPTED: &[u8] = include_bytes!("fixtures/encrypted_marker.pdf");

/// Test stand-in for cia-mozjpeg.
struct TestEncoder;

impl JpegEncoder for TestEncoder {
    fn encode_rgb(&self, rgb: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String> {
        encode(rgb, w, h, quality, jpeg_encoder::ColorType::Rgb)
    }
    fn encode_gray(&self, gray: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String> {
        encode(gray, w, h, quality, jpeg_encoder::ColorType::Luma)
    }
}

fn encode(
    px: &[u8],
    w: u32,
    h: u32,
    quality: u8,
    ct: jpeg_encoder::ColorType,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.encode(px, w as u16, h as u16, ct)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn never() -> bool {
    false
}

fn page_count(bytes: &[u8]) -> usize {
    Document::load_mem(bytes).expect("loads").get_pages().len()
}

fn fit(budget: u64) -> PdfOptions {
    PdfOptions {
        budget_bytes: Some(budget),
        ..Default::default()
    }
}

fn smaller(level: SmallerLevel) -> PdfOptions {
    PdfOptions {
        smaller_mode: Some(level),
        ..Default::default()
    }
}

#[test]
fn inspect_reports_pages_images_and_encryption() {
    let info = inspect(SCAN).unwrap();
    assert_eq!(info.page_count, 5);
    assert_eq!(info.image_count, 5);
    assert!(!info.encrypted);
    assert_eq!(info.version, "1.4");

    let info = inspect(ENCRYPTED).unwrap();
    assert!(info.encrypted);

    let info = inspect(VECTOR).unwrap();
    assert_eq!(info.page_count, 3);
    assert_eq!(info.image_count, 0);
}

#[test]
fn scan_photos_fits_600kb_and_verifies() {
    let out = optimise(SCAN, &fit(600_000), &TestEncoder, &never).unwrap();
    let PdfOutcome::Done(r) = out else {
        panic!("expected Done, got {out}");
    };
    assert!(r.bytes.len() <= 600_000, "{} bytes", r.bytes.len());
    assert!(r.step >= 1, "needs the image pass: {:?}", r.attempts);
    assert_eq!(r.images_recompressed, 5);
    assert_eq!(r.images_skipped, 0);
    assert!(r.lossless_bytes > 600_000);
    assert!(r
        .attempts
        .iter()
        .any(|a| a.step == 0 && a.quality.is_none()));
    assert_eq!(page_count(&r.bytes), 5);
    let report = verify(SCAN, &r.bytes, Some(600_000)).unwrap();
    assert!(report.ok, "{report}");
    assert_eq!(report.page_count, 5);
    assert!(report
        .checks
        .iter()
        .any(|c| c.name == "content_streams_unchanged" && c.passed));
    assert!(report
        .checks
        .iter()
        .any(|c| c.name == "images_decode" && c.passed));
    assert_eq!(r.verification, report);
}

#[test]
fn scan_photos_refuses_5kb() {
    let out = optimise(SCAN, &fit(5_000), &TestEncoder, &never).unwrap();
    match out {
        PdfOutcome::Refused { smallest_bytes } => {
            assert!(smallest_bytes > 5_000);
            assert!(
                smallest_bytes < SCAN.len() as u64 / 4,
                "ladder bottom should be far smaller: {smallest_bytes}"
            );
        }
        other => panic!("expected Refused, got {other}"),
    }
}

#[test]
fn scan_photos_already_fitting_takes_lossless_unless_q85_saves_ten_percent() {
    let out = optimise(SCAN, &fit(u64::MAX), &TestEncoder, &never).unwrap();
    match out {
        PdfOutcome::Done(r) => {
            assert!(r.step <= 1);
            assert_eq!(page_count(&r.bytes), 5);
            if r.step == 1 {
                assert!((r.bytes.len() as u64) * 10 < r.lossless_bytes * 9);
            }
        }
        PdfOutcome::KeptOriginal { .. } => {}
        other => panic!("unexpected {other}"),
    }
}

#[test]
fn vector_text_smaller_mode_is_lossless() {
    let out = optimise(
        VECTOR,
        &smaller(SmallerLevel::KeepQuality),
        &TestEncoder,
        &never,
    )
    .unwrap();
    let bytes = match &out {
        PdfOutcome::Done(r) => {
            assert!(r.bytes.len() < VECTOR.len());
            assert_eq!(r.step, 0);
            assert_eq!(r.images_recompressed, 0);
            assert_eq!(page_count(&r.bytes), 3);
            r.bytes.clone()
        }
        PdfOutcome::KeptOriginal { .. } => VECTOR.to_vec(),
        other => panic!("unexpected {other}"),
    };
    // Content streams are byte-identical after inflating.
    let orig = Document::load_mem(VECTOR).unwrap();
    let new = Document::load_mem(&bytes).unwrap();
    let orig_pages = orig.get_pages();
    let new_pages = new.get_pages();
    assert_eq!(orig_pages.len(), new_pages.len());
    for (num, orig_id) in &orig_pages {
        let new_id = new_pages[num];
        let o: Vec<u8> = orig
            .get_page_contents(*orig_id)
            .iter()
            .flat_map(|id| {
                orig.get_object(*id)
                    .unwrap()
                    .as_stream()
                    .unwrap()
                    .get_plain_content()
                    .unwrap()
            })
            .collect();
        let n: Vec<u8> = new
            .get_page_contents(new_id)
            .iter()
            .flat_map(|id| {
                new.get_object(*id)
                    .unwrap()
                    .as_stream()
                    .unwrap()
                    .get_plain_content()
                    .unwrap()
            })
            .collect();
        assert!(!o.is_empty());
        assert_eq!(o, n, "page {num} content changed");
    }
    assert!(verify(VECTOR, &bytes, None).unwrap().ok);
}

#[test]
fn encrypted_marker_is_refused_up_front() {
    let out = optimise(ENCRYPTED, &fit(1_000), &TestEncoder, &never).unwrap();
    assert_eq!(out, PdfOutcome::Encrypted);
    let out = optimise(
        ENCRYPTED,
        &smaller(SmallerLevel::Smallest),
        &TestEncoder,
        &never,
    )
    .unwrap();
    assert_eq!(out, PdfOutcome::Encrypted);
}

#[test]
fn truncated_pdf_is_damaged() {
    for cut in [16usize, 300, 2_000] {
        let err = optimise(&SCAN[..cut], &fit(600_000), &TestEncoder, &never).unwrap_err();
        assert!(matches!(err, PdfError::Damaged(_)), "cut at {cut}: {err:?}");
        assert!(matches!(inspect(&SCAN[..cut]), Err(PdfError::Damaged(_))));
    }
    assert!(matches!(
        optimise(b"", &fit(1), &TestEncoder, &never),
        Err(PdfError::Damaged(_))
    ));
    assert!(matches!(
        optimise(b"%PDF-1.4\nhello", &fit(1), &TestEncoder, &never),
        Err(PdfError::Damaged(_))
    ));
}

#[test]
fn cancel_is_honoured() {
    let err = optimise(SCAN, &fit(600_000), &TestEncoder, &|| true).unwrap_err();
    assert_eq!(err, PdfError::Cancelled);
}

#[test]
fn verify_fails_on_corrupted_output() {
    let PdfOutcome::Done(r) = optimise(SCAN, &fit(600_000), &TestEncoder, &never).unwrap() else {
        panic!("expected Done");
    };
    let good = r.bytes;
    assert!(verify(SCAN, &good, Some(600_000)).unwrap().ok);

    // Over the limit.
    let report = verify(SCAN, &good, Some(good.len() as u64 - 1)).unwrap();
    assert!(!report.ok);
    assert!(report.failures().any(|c| c.name == "size"));

    // Truncated: fewer pages or no load at all.
    let report = verify(SCAN, &good[..good.len() / 2], Some(600_000)).unwrap();
    assert!(!report.ok, "{report}");

    // Scribble over the first image stream's JPEG data.
    let mut bad = good.clone();
    let soi = bad
        .windows(2)
        .position(|w| w == [0xFF, 0xD8])
        .expect("a JPEG SOI");
    for b in &mut bad[soi + 200..soi + 2_000] {
        *b = 0;
    }
    let report = verify(SCAN, &bad, Some(600_000)).unwrap();
    assert!(!report.ok, "{report}");
    assert!(
        report
            .failures()
            .any(|c| c.name == "images_decode" || c.name == "loads"),
        "{report}"
    );

    // Edit a page's content stream: still a valid PDF, but the text changed.
    let mut doc = Document::load_mem(&good).unwrap();
    let first_page = *doc.get_pages().values().next().unwrap();
    let content_id = doc.get_page_contents(first_page)[0];
    let stream = doc
        .get_object_mut(content_id)
        .unwrap()
        .as_stream_mut()
        .unwrap();
    let mut text = stream.get_plain_content().unwrap();
    text.extend_from_slice(b"\n0 0 m 1 1 l S\n");
    stream.set_plain_content(text);
    let mut edited = Vec::new();
    doc.save_to(&mut edited).unwrap();
    let report = verify(SCAN, &edited, None).unwrap();
    assert!(!report.ok);
    assert!(
        report
            .failures()
            .any(|c| c.name == "content_streams_unchanged"),
        "{report}"
    );

    // The original itself must be readable.
    assert!(matches!(
        verify(&SCAN[..100], &good, None),
        Err(PdfError::Damaged(_))
    ));
}

// ---- synthetic documents for the Flate image paths and metadata handling ----

fn noise(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9E37_79B9;
    (0..len)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // Smooth gradient plus noise so it compresses like a photo.
            ((i % 251) as u32 / 2 + (x & 0x7f)) as u8
        })
        .collect()
}

/// `(name, width, height, pixels, optional soft mask)`.
type SynthImage<'a> = (&'a str, u32, u32, Vec<u8>, Option<Vec<u8>>);

/// One page drawing `images` edge to edge, each a Flate image XObject.
fn synth_pdf(images: Vec<SynthImage<'_>>, with_metadata: bool) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let mut xobjects = dictionary! {};
    let mut content = String::new();
    for (name, w, h, rgb, smask) in images {
        let mut dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => w as i64,
            "Height" => h as i64,
            "ColorSpace" => if rgb.len() == (w * h * 3) as usize { "DeviceRGB" } else { "DeviceGray" },
            "BitsPerComponent" => 8,
        };
        if let Some(mask) = smask {
            let mut ms = Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => w as i64,
                    "Height" => h as i64,
                    "ColorSpace" => "DeviceGray",
                    "BitsPerComponent" => 8,
                },
                mask,
            );
            ms.compress().unwrap();
            let mid = doc.add_object(ms);
            dict.set("SMask", Object::Reference(mid));
        }
        let mut s = Stream::new(dict, rgb);
        s.compress().unwrap();
        let id = doc.add_object(s);
        xobjects.set(name, Object::Reference(id));
        content.push_str(&format!("q {w} 0 0 {h} 0 0 cm /{name} Do Q\n"));
    }
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let resources_id = doc.add_object(dictionary! { "XObject" => xobjects });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }),
    );
    let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
    if with_metadata {
        let xmp = Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            b"<?xpacket begin='' id='W5M0MpCehiHzreSzNTczkc9d'?><x:xmpmeta xmlns:x='adobe:ns:meta/'/>".to_vec(),
        );
        let mid = doc.add_object(xmp);
        catalog.set("Metadata", Object::Reference(mid));
    }
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn image_filters(bytes: &[u8]) -> Vec<(String, i64, i64)> {
    let doc = Document::load_mem(bytes).unwrap();
    let mut out: Vec<_> = doc
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Subtype").and_then(|o| o.as_name()).ok() == Some(b"Image"))
        .map(|s| {
            (
                s.dict
                    .get(b"Filter")
                    .and_then(|o| o.as_name())
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default(),
                s.dict.get(b"Width").and_then(|o| o.as_i64()).unwrap(),
                s.dict.get(b"Height").and_then(|o| o.as_i64()).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn flate_photo_becomes_jpeg_and_flate_graphic_stays_flate() {
    let (w, h) = (640u32, 480u32);
    let photo = noise((w * h * 3) as usize);
    let graphic: Vec<u8> = (0..(w * h) as usize)
        .map(|i| if (i / 64) % 2 == 0 { 20 } else { 230 })
        .collect();
    let pdf = synth_pdf(
        vec![
            ("Photo", w, h, photo, None),
            ("Graphic", w, h, graphic, None),
        ],
        false,
    );
    let out = optimise(&pdf, &smaller(SmallerLevel::Smallest), &TestEncoder, &never).unwrap();
    let PdfOutcome::Done(r) = out else {
        panic!("expected Done, got {out}");
    };
    assert_eq!(r.images_recompressed, 1, "{r:?}");
    assert_eq!(r.images_skipped, 1);
    assert!(
        r.bytes.len() < pdf.len() / 2,
        "{} vs {}",
        r.bytes.len(),
        pdf.len()
    );
    assert_eq!(page_count(&r.bytes), 1);
    let filters = image_filters(&r.bytes);
    assert_eq!(
        filters,
        vec![
            ("DCTDecode".to_string(), 640, 480),
            ("FlateDecode".to_string(), 640, 480)
        ]
    );
    assert!(verify(&pdf, &r.bytes, None).unwrap().ok);
}

#[test]
fn pixel_cap_downscales_base_and_soft_mask() {
    let (w, h) = (2400u32, 600u32);
    let photo = noise((w * h * 3) as usize);
    let mask: Vec<u8> = (0..(w * h) as usize).map(|i| (i % 256) as u8).collect();
    let pdf = synth_pdf(vec![("Im", w, h, photo, Some(mask))], false);
    // Tiny budget forces the ladder down to the 1200 px cap.
    let out = optimise(&pdf, &fit(60_000), &TestEncoder, &never).unwrap();
    let step = match &out {
        PdfOutcome::Done(r) => {
            assert!(r.bytes.len() <= 60_000);
            assert_eq!(page_count(&r.bytes), 1);
            let filters = image_filters(&r.bytes);
            let cap = r.pixel_cap.expect("a capped step") as i64;
            assert!(filters.iter().all(|(_, fw, _)| *fw == cap), "{filters:?}");
            assert!(filters.iter().any(|(f, _, _)| f == "DCTDecode"));
            assert!(
                filters.iter().any(|(f, _, _)| f == "FlateDecode"),
                "mask stays Flate: {filters:?}"
            );
            assert!(verify(&pdf, &r.bytes, Some(60_000)).unwrap().ok);
            r.step
        }
        PdfOutcome::Refused { smallest_bytes } => panic!("refused at {smallest_bytes}"),
        other => panic!("unexpected {other}"),
    };
    assert!(step >= 2, "a 2400 px image must need a capped step: {step}");
}

#[test]
fn xmp_metadata_is_dropped_unless_kept() {
    let pdf = synth_pdf(vec![("Im", 256, 256, noise(256 * 256 * 3), None)], true);
    assert!(Document::load_mem(&pdf)
        .unwrap()
        .catalog()
        .unwrap()
        .has(b"Metadata"));

    let has_metadata = |bytes: &[u8]| -> bool {
        let doc = Document::load_mem(bytes).unwrap();
        let in_catalog = doc.catalog().unwrap().has(b"Metadata");
        let stream_present = doc.objects.values().any(|o| {
            o.as_stream()
                .map(|s| s.dict.get(b"Type").and_then(|t| t.as_name()).ok() == Some(b"Metadata"))
                .unwrap_or(false)
        });
        assert_eq!(in_catalog, stream_present);
        in_catalog
    };

    let out = optimise(&pdf, &smaller(SmallerLevel::Smallest), &TestEncoder, &never).unwrap();
    let PdfOutcome::Done(r) = out else {
        panic!("{out}")
    };
    assert!(!has_metadata(&r.bytes));

    let opts = PdfOptions {
        keep_document_details: true,
        smaller_mode: Some(SmallerLevel::Smallest),
        ..Default::default()
    };
    let out = optimise(&pdf, &opts, &TestEncoder, &never).unwrap();
    let PdfOutcome::Done(r) = out else {
        panic!("{out}")
    };
    assert!(has_metadata(&r.bytes));
}
