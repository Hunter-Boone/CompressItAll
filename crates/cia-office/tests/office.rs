//! Round trips over the synthetic fixtures, in-test ODF/EPUB packages, and
//! deliberately broken inputs and outputs (DESIGN.md 7.1).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use cia_archive::zipfmt::{ZipWriter, METHOD_DEFLATE, METHOD_STORED};
use cia_office::{
    detect, inspect, optimise, verify, ImageFormat, MediaKind, MediaRecompressor, Mode,
    OfficeError, OfficeKind, OfficeOptions, OfficeOutcome, OfficeResult, QualityLabel,
    Recompressed,
};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, RgbImage};

const REPORT: &[u8] = include_bytes!("fixtures/report.docx");
const DECK: &[u8] = include_bytes!("fixtures/deck.pptx");

const CFB: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn never() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Test stand-in for the image, video and audio planners.

/// What the stub does with videos.
#[derive(Clone, Copy)]
enum VideoStub {
    /// "No FFmpeg": always leave the video alone.
    NoOp,
    /// Pretend to be the video planner: hand back a prefix that fits the
    /// budget. Verification does not decode video, so this is enough to
    /// exercise the allocator.
    Truncate,
}

struct TestRecompressor {
    video: VideoStub,
    /// Return `None` for every image too (for KeptOriginal tests).
    inert: bool,
    /// Decoded inputs by CRC: the planner asks about each image several
    /// times (keep-quality probe, floor probe, fit, round 2).
    decoded: RefCell<HashMap<u32, DynamicImage>>,
}

impl TestRecompressor {
    fn new(video: VideoStub, inert: bool) -> Self {
        Self {
            video,
            inert,
            decoded: RefCell::new(HashMap::new()),
        }
    }
    fn images_only() -> Self {
        Self::new(VideoStub::NoOp, false)
    }
    fn with_video() -> Self {
        Self::new(VideoStub::Truncate, false)
    }
    fn inert() -> Self {
        Self::new(VideoStub::NoOp, true)
    }
    fn decode(&self, bytes: &[u8]) -> Result<DynamicImage, String> {
        let key = crc32fast::hash(bytes);
        if let Some(img) = self.decoded.borrow().get(&key) {
            return Ok(img.clone());
        }
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        self.decoded.borrow_mut().insert(key, img.clone());
        Ok(img)
    }
}

fn label_for_quality(q: u8) -> QualityLabel {
    match q {
        80.. => QualityLabel::Great,
        60..=79 => QualityLabel::Good,
        _ => QualityLabel::Okay,
    }
}

/// JPEG encodes by (source CRC, quality), shared across tests: the planner
/// asks for the same photos at the same qualities many times over.
type JpegCache = Mutex<HashMap<(u32, u8), Vec<u8>>>;

fn jpeg_cache() -> &'static JpegCache {
    static CACHE: OnceLock<JpegCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn encode_jpeg(img: &DynamicImage, quality: u8) -> Vec<u8> {
    let key = (crc32fast::hash(img.as_bytes()), quality);
    if let Some(hit) = jpeg_cache().lock().unwrap().get(&key) {
        return hit.clone();
    }
    let mut out = Vec::new();
    let enc = JpegEncoder::new_with_quality(&mut out, quality);
    img.to_rgb8().write_with_encoder(enc).expect("jpeg encode");
    jpeg_cache().lock().unwrap().insert(key, out.clone());
    out
}

fn encode_png(img: &DynamicImage, compression: CompressionType) -> Vec<u8> {
    let mut out = Vec::new();
    let enc = PngEncoder::new_with_quality(&mut out, compression, FilterType::Adaptive);
    img.write_with_encoder(enc).expect("png encode");
    out
}

const JPEG_KEEP: u8 = 85;
const JPEG_FLOOR: u8 = 35;

impl MediaRecompressor for TestRecompressor {
    fn recompress_image(
        &self,
        bytes: &[u8],
        format: ImageFormat,
        budget: Option<u64>,
        mode: Mode,
        _cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String> {
        if self.inert {
            return Ok(None);
        }
        let img = self.decode(bytes)?;
        match format {
            ImageFormat::Png => {
                let out = encode_png(&img, CompressionType::Best);
                if budget.is_some_and(|b| out.len() as u64 > b) {
                    return Ok(None);
                }
                Ok(Some(Recompressed {
                    bytes: out,
                    quality_label: QualityLabel::Great,
                }))
            }
            ImageFormat::Jpeg => {
                let quality = match (mode, budget) {
                    (Mode::SmallerKeepQuality, _) => JPEG_KEEP,
                    (Mode::SmallerSmallest, _) => JPEG_FLOOR,
                    (Mode::Fit, None) => JPEG_KEEP,
                    (Mode::Fit, Some(b)) => {
                        // Largest quality in {35, 40, .., 95} whose output fits b.
                        let steps: Vec<u8> = (JPEG_FLOOR..=95).step_by(5).collect();
                        if encode_jpeg(&img, steps[0]).len() as u64 > b {
                            return Ok(None);
                        }
                        let (mut lo, mut hi) = (0usize, steps.len() - 1);
                        while lo < hi {
                            let mid = (lo + hi).div_ceil(2);
                            if encode_jpeg(&img, steps[mid]).len() as u64 <= b {
                                lo = mid;
                            } else {
                                hi = mid - 1;
                            }
                        }
                        steps[lo]
                    }
                };
                Ok(Some(Recompressed {
                    bytes: encode_jpeg(&img, quality),
                    quality_label: label_for_quality(quality),
                }))
            }
        }
    }

    fn recompress_video(
        &self,
        bytes: &[u8],
        ext: &str,
        budget: Option<u64>,
        _cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String> {
        assert_eq!(ext, "mp4");
        match (self.video, budget) {
            (VideoStub::Truncate, Some(b)) if (b as usize) < bytes.len() => {
                Ok(Some(Recompressed {
                    bytes: bytes[..b as usize].to_vec(),
                    quality_label: QualityLabel::Okay,
                }))
            }
            _ => Ok(None),
        }
    }

    fn recompress_audio(
        &self,
        _bytes: &[u8],
        _ext: &str,
        _budget: Option<u64>,
        _cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String> {
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Synthetic packages.

/// Write a ZIP with every entry stored (so the planner has container
/// savings to find) in exactly the order given.
fn zip_stored(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zw = ZipWriter::new(Vec::new(), None);
    for (name, data) in entries {
        zw.write_entry(
            name,
            METHOD_STORED,
            crc32fast::hash(data),
            data.len() as u64,
            data,
            false,
        )
        .unwrap();
    }
    zw.finish().unwrap()
}

/// A 256x256 PNG with soft gradients, encoded fast so there is slack for the
/// planner to recover losslessly.
fn test_png() -> Vec<u8> {
    let img = RgbImage::from_fn(256, 256, |x, y| {
        image::Rgb([(x as u8), (y as u8), ((x / 2 + y / 2) as u8)])
    });
    encode_png(&DynamicImage::ImageRgb8(img), CompressionType::Fast)
}

fn filler_xml(paragraphs: usize) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text>"#,
    );
    for i in 0..paragraphs {
        s.push_str(&format!(
            "<text:p text:style-name=\"P{}\">Paragraph number {i} of the synthetic document body.</text:p>",
            i % 7
        ));
    }
    s.push_str("</office:text></office:body></office:document-content>");
    s
}

const ODT_MIMETYPE: &[u8] = b"application/vnd.oasis.opendocument.text";

fn odt_manifest() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Pictures/logo.png" manifest:media-type="image/png"/></manifest:manifest>"#
}

const ODT_META: &str = r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><office:meta><meta:initial-creator>Ada Lovelace</meta:initial-creator><dc:creator>Ada Lovelace</dc:creator><dc:title>Notes on the Engine</dc:title><meta:editing-cycles>12</meta:editing-cycles></office:meta></office:document-meta>"#;

/// Minimal ODT. `mimetype_first` false puts it second, which a compliant
/// writer never does but a repack must still fix.
fn make_odt(mimetype_first: bool, mimetype: &[u8]) -> Vec<u8> {
    let content = filler_xml(400);
    let png = test_png();
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("content.xml", content.as_bytes()),
        ("meta.xml", ODT_META.as_bytes()),
        ("META-INF/manifest.xml", odt_manifest().as_bytes()),
        ("Pictures/logo.png", &png),
    ];
    let pos = if mimetype_first { 0 } else { 1 };
    entries.insert(pos, ("mimetype", mimetype));
    zip_stored(&entries)
}

fn make_epub() -> Vec<u8> {
    let container = r#"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:uuid:1</dc:identifier><dc:title>Synthetic</dc:title><dc:language>en</dc:language></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/chapter1.xhtml" media-type="application/xhtml+xml"/><item id="img" href="images/cover%20art.png" media-type="image/png"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
    let nav = r#"<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>nav</title></head><body><nav epub:type="toc"><ol><li><a href="text/chapter1.xhtml">One</a></li></ol></nav></body></html>"#;
    let chapter = filler_xml(300).replace("office:document-content", "html");
    let png = test_png();
    zip_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/nav.xhtml", nav.as_bytes()),
        ("OEBPS/text/chapter1.xhtml", chapter.as_bytes()),
        ("OEBPS/images/cover art.png", &png),
    ])
}

const CORE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Budget 2026</dc:title><dc:creator>Grace Hopper</dc:creator><cp:lastModifiedBy>Grace Hopper</cp:lastModifiedBy><cp:revision>2</cp:revision><dcterms:created xsi:type="dcterms:W3CDTF">2026-01-02T03:04:05Z</dcterms:created></cp:coreProperties>"#;

/// Minimal OOXML package whose main part content type is `main_ct`, with a
/// core.xml and one PNG under `dir/media/`, plus optional vbaProject.bin.
fn make_ooxml(main_ct: &str, dir: &str, main_part: &str, with_vba: bool) -> Vec<u8> {
    let content_types = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/{dir}/{main_part}" ContentType="{main_ct}"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#
    );
    let root_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{dir}/{main_part}"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
    );
    let main_rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#;
    let main = filler_xml(200);
    let png = test_png();
    let vba = vec![0xD0u8; 64];
    let rels_name = format!("{dir}/_rels/{main_part}.rels");
    let main_name = format!("{dir}/{main_part}");
    let media_name = format!("{dir}/media/image1.png");
    let vba_name = format!("{dir}/vbaProject.bin");
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", root_rels.as_bytes()),
        (&rels_name, main_rels.as_bytes()),
        (&main_name, main.as_bytes()),
        (&media_name, &png),
        ("docProps/core.xml", CORE_XML.as_bytes()),
    ];
    if with_vba {
        entries.push((&vba_name, &vba));
    }
    zip_stored(&entries)
}

fn fit(budget: u64, host_can_video: bool) -> OfficeOptions {
    OfficeOptions {
        budget_bytes: Some(budget),
        mode: Mode::Fit,
        keep_document_details: true,
        host_can_video,
    }
}

fn smaller(mode: Mode) -> OfficeOptions {
    OfficeOptions {
        budget_bytes: None,
        mode,
        keep_document_details: true,
        host_can_video: false,
    }
}

fn done(outcome: OfficeOutcome) -> OfficeResult {
    match outcome {
        OfficeOutcome::Done(r) => r,
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Entries of a zip as `(name, method, bytes)` via the `zip` crate, which is
/// an independent reader from the one cia-archive wraps for verification.
fn list(bytes: &[u8]) -> Vec<(String, u16, Vec<u8>)> {
    let mut archive = cia_archive::open(bytes).unwrap();
    let names: Vec<(String, String)> = archive
        .entries()
        .iter()
        .map(|e| (e.name.clone(), e.method.clone()))
        .collect();
    let data = archive.extract_all().unwrap();
    names
        .into_iter()
        .zip(data)
        .map(|((n, m), d)| (n, if m == "stored" { 0 } else { 8 }, d))
        .collect()
}

fn entry<'a>(entries: &'a [(String, u16, Vec<u8>)], name: &str) -> &'a (String, u16, Vec<u8>) {
    entries
        .iter()
        .find(|(n, _, _)| n == name)
        .unwrap_or_else(|| panic!("no entry {name}"))
}

// ---------------------------------------------------------------------------
// Detection and inspection.

#[test]
fn detect_each_kind() {
    assert_eq!(detect(REPORT).unwrap(), Some(OfficeKind::Docx));
    assert_eq!(detect(DECK).unwrap(), Some(OfficeKind::Pptx));

    let docm = make_ooxml(
        "application/vnd.ms-word.document.macroEnabled.main+xml",
        "word",
        "document.xml",
        true,
    );
    assert_eq!(detect(&docm).unwrap(), Some(OfficeKind::Docm));
    let pptm = make_ooxml(
        "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml",
        "ppt",
        "presentation.xml",
        true,
    );
    assert_eq!(detect(&pptm).unwrap(), Some(OfficeKind::Pptm));
    let xlsx = make_ooxml(
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
        "xl",
        "workbook.xml",
        false,
    );
    assert_eq!(detect(&xlsx).unwrap(), Some(OfficeKind::Xlsx));
    let xlsm = make_ooxml(
        "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        "xl",
        "workbook.xml",
        true,
    );
    assert_eq!(detect(&xlsm).unwrap(), Some(OfficeKind::Xlsm));

    assert_eq!(
        detect(&make_odt(true, ODT_MIMETYPE)).unwrap(),
        Some(OfficeKind::Odt)
    );
    assert_eq!(
        detect(&make_odt(
            true,
            b"application/vnd.oasis.opendocument.presentation"
        ))
        .unwrap(),
        Some(OfficeKind::Odp)
    );
    assert_eq!(
        detect(&make_odt(
            true,
            b"application/vnd.oasis.opendocument.spreadsheet"
        ))
        .unwrap(),
        Some(OfficeKind::Ods)
    );
    assert_eq!(detect(&make_epub()).unwrap(), Some(OfficeKind::Epub));

    // Not Office: a plain zip, a PDF, nothing.
    let plain = zip_stored(&[("readme.txt", b"hello")]);
    assert_eq!(detect(&plain).unwrap(), None);
    assert_eq!(detect(b"%PDF-1.7 ...").unwrap(), None);
    assert_eq!(detect(&[]).unwrap(), None);
}

#[test]
fn ole_compound_file_is_encrypted() {
    let mut cfb = CFB.to_vec();
    cfb.extend_from_slice(&[0u8; 4096]);
    assert_eq!(detect(&cfb), Err(OfficeError::Encrypted));
    assert_eq!(inspect(&cfb).unwrap_err(), OfficeError::Encrypted);
    let err = optimise(
        &cfb,
        &fit(1_000_000, false),
        &TestRecompressor::images_only(),
        &never,
    )
    .unwrap_err();
    assert_eq!(err, OfficeError::Encrypted);
    assert_eq!(err.to_string(), "This document is password-protected");
}

#[test]
fn inspect_lists_media_and_fixed_bytes() {
    let info = inspect(DECK).unwrap();
    assert_eq!(info.kind, OfficeKind::Pptx);
    assert_eq!(info.entries.len(), 15);
    assert_eq!(info.media.len(), 11);
    let videos = info
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Video)
        .count();
    let jpegs = info
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Image(ImageFormat::Jpeg))
        .count();
    assert_eq!((videos, jpegs), (1, 10));
    assert_eq!(
        info.media
            .iter()
            .find(|m| m.kind == MediaKind::Video)
            .unwrap()
            .name,
        "ppt/media/media1.mp4"
    );
    // Four small XML parts plus container overhead: well under 10 KB.
    assert!(info.fixed_bytes_estimate > 1000 && info.fixed_bytes_estimate < 10_000);

    let info = inspect(REPORT).unwrap();
    assert_eq!(info.kind, OfficeKind::Docx);
    assert_eq!(info.media.len(), 7);
    assert!(info
        .media
        .iter()
        .any(|m| m.kind == MediaKind::Image(ImageFormat::Png)));

    let info = inspect(&make_epub()).unwrap();
    assert_eq!(info.media.len(), 1);
    assert_eq!(info.media[0].name, "OEBPS/images/cover art.png");
    assert_eq!(info.media[0].kind, MediaKind::Image(ImageFormat::Png));
}

// ---------------------------------------------------------------------------
// Fit mode.

#[test]
fn deck_fits_3mb_budget_and_verifies() {
    let budget = 3_000_000u64;
    let r = done(
        optimise(
            DECK,
            &fit(budget, true),
            &TestRecompressor::with_video(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.bytes.len() as u64 <= budget, "{} bytes", r.bytes.len());
    assert!(r.verification.ok, "{}", r.verification);
    assert_eq!(r.media_recompressed, 11);
    assert_eq!(r.media_left, 0);
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    assert_eq!(r.predicted_quality, QualityLabel::Okay);

    // Independent re-verification and structure checks.
    let report = verify(DECK, &r.bytes, Some(budget));
    assert!(report.ok, "{report}");
    let out = list(&r.bytes);
    let orig = list(DECK);
    assert_eq!(
        out.iter().map(|e| &e.0).collect::<Vec<_>>(),
        orig.iter().map(|e| &e.0).collect::<Vec<_>>()
    );
    // XML untouched; images smaller and still JPEG; the video, which
    // deflate cannot shrink by one percent, is stored.
    for (name, _, data) in &orig {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_eq!(&entry(&out, name).2, data, "{name} changed");
        }
    }
    for (name, method, data) in &out {
        if name.ends_with(".jpeg") {
            assert!(data.starts_with(&[0xFF, 0xD8, 0xFF]), "{name}");
            assert!(
                data.len() < entry(&orig, name).2.len(),
                "{name} did not shrink"
            );
        }
        if name.ends_with(".mp4") {
            assert_eq!(*method, METHOD_STORED);
        }
    }

    python_checks(&r.bytes);
}

/// Open the output with Python's zipfile and xml.etree: CRCs, XML
/// well-formedness, and every internal relationship target resolving.
fn python_checks(bytes: &[u8]) {
    let Ok(which) = Command::new("which").arg("python3").output() else {
        eprintln!("skipping python check: `which` not available");
        return;
    };
    if !which.status.success() {
        eprintln!("skipping python check: python3 not found");
        return;
    }
    let path = std::env::temp_dir().join(format!(
        "cia-office-{}-{}.zip",
        std::process::id(),
        bytes.len()
    ));
    std::fs::write(&path, bytes).unwrap();
    const SCRIPT: &str = r#"
import sys, zipfile, posixpath
import xml.etree.ElementTree as ET
z = zipfile.ZipFile(sys.argv[1])
bad = z.testzip()
assert bad is None, f"CRC failed on {bad}"
names = set(z.namelist())
rels = 0
for info in z.infolist():
    n = info.filename
    if n.endswith('.xml') or n.endswith('.rels'):
        root = ET.fromstring(z.read(n))
    if n.endswith('.rels'):
        base = n[:n.index('_rels/')]
        for rel in root.iter('{http://schemas.openxmlformats.org/package/2006/relationships}Relationship'):
            if rel.get('TargetMode') == 'External':
                continue
            t = rel.get('Target')
            full = t[1:] if t.startswith('/') else posixpath.normpath(posixpath.join(base, t))
            assert full in names, f"{n}: {t} -> {full} missing"
            rels += 1
assert rels > 0, "no relationships checked"
print(f"ok: {len(names)} entries, {rels} relationships resolve")
"#;
    let out = Command::new("python3")
        .arg("-c")
        .arg(SCRIPT)
        .arg(&path)
        .output()
        .expect("run python3");
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "python check failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("relationships resolve"), "{stdout}");
}

#[test]
fn deck_without_video_support_leaves_video_and_says_so() {
    // 5.6 MB video stays; the ten photos must share what is left of 9 MB.
    let budget = 9_000_000u64;
    let r = done(
        optimise(
            DECK,
            &fit(budget, false),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.bytes.len() as u64 <= budget);
    assert_eq!(r.notes, vec!["1 video left as is".to_owned()]);
    assert_eq!(r.media_recompressed, 10);
    assert_eq!(r.media_left, 1);
    let out = list(&r.bytes);
    let orig = list(DECK);
    assert_eq!(
        entry(&out, "ppt/media/media1.mp4").2,
        entry(&orig, "ppt/media/media1.mp4").2
    );

    // Host can do video but the planner declines: same note.
    let r = done(
        optimise(
            DECK,
            &fit(budget, true),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert_eq!(r.notes, vec!["1 video left as is".to_owned()]);
}

#[test]
fn budget_below_floor_refuses_with_smallest_bytes() {
    // 3 MB with an untouchable 5.6 MB video: refused, and the floor it
    // reports is at least the video plus the fixed parts.
    let out = optimise(
        DECK,
        &fit(3_000_000, false),
        &TestRecompressor::images_only(),
        &never,
    )
    .unwrap();
    let OfficeOutcome::Refused { smallest_bytes } = out else {
        panic!("expected Refused, got {out:?}");
    };
    assert!(smallest_bytes > 5_605_361, "{smallest_bytes}");
    assert!(smallest_bytes < DECK.len() as u64, "{smallest_bytes}");

    // The report's photos at the floor still do not fit 100 kB.
    let out = optimise(
        REPORT,
        &fit(100_000, false),
        &TestRecompressor::images_only(),
        &never,
    )
    .unwrap();
    let OfficeOutcome::Refused { smallest_bytes } = out else {
        panic!("expected Refused, got {out:?}");
    };
    assert!(smallest_bytes > 100_000);
    assert!(smallest_bytes < REPORT.len() as u64);

    // A budget just above the reported floor fits.
    let r = done(
        optimise(
            REPORT,
            &fit(smallest_bytes + 20_000, false),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.bytes.len() as u64 <= smallest_bytes + 20_000);
    assert!(r.verification.ok, "{}", r.verification);
}

#[test]
fn fit_takes_lossless_when_it_fits_and_keeps_original_when_nothing_helps() {
    // Generous budget: every image gets its keep-quality encoding.
    let r = done(
        optimise(
            REPORT,
            &fit(10_000_000, false),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.bytes.len() < REPORT.len());
    assert_eq!(r.predicted_quality, QualityLabel::Great);

    // Inert recompressor and an already-deflated package: nothing to gain.
    let out = optimise(
        REPORT,
        &fit(10_000_000, false),
        &TestRecompressor::inert(),
        &never,
    )
    .unwrap();
    assert!(matches!(out, OfficeOutcome::KeptOriginal { .. }), "{out:?}");
}

// ---------------------------------------------------------------------------
// Smaller mode.

#[test]
fn docx_smaller_mode_shrinks_and_verifies() {
    let keep = done(
        optimise(
            REPORT,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(
        keep.bytes.len() < REPORT.len() * 95 / 100,
        "{}",
        keep.bytes.len()
    );
    assert!(keep.verification.ok, "{}", keep.verification);
    assert_eq!(keep.media_recompressed, 7);
    assert_eq!(keep.predicted_quality, QualityLabel::Great);
    assert!(verify(REPORT, &keep.bytes, None).ok);
    assert_eq!(detect(&keep.bytes).unwrap(), Some(OfficeKind::Docx));

    let smallest = done(
        optimise(
            REPORT,
            &smaller(Mode::SmallerSmallest),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(smallest.bytes.len() < keep.bytes.len());
    assert_eq!(smallest.predicted_quality, QualityLabel::Okay);

    let out = list(&keep.bytes);
    let png = entry(&out, "word/media/image9.png");
    assert!(png.2.starts_with(b"\x89PNG"));
    assert_eq!(entry(&out, "word/document.xml").1, METHOD_DEFLATE);
    // The original mp4 in the deck cannot be deflated by one percent: stored.
    let deck = done(
        optimise(
            DECK,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert_eq!(
        entry(&list(&deck.bytes), "ppt/media/media1.mp4").1,
        METHOD_STORED
    );
    assert_eq!(deck.notes, vec!["1 video left as is".to_owned()]);
}

// ---------------------------------------------------------------------------
// ODF and EPUB.

fn assert_mimetype_first_stored(bytes: &[u8]) {
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    assert_eq!(&bytes[8..10], &[0, 0], "method must be Stored");
    assert_eq!(&bytes[30..38], b"mimetype");
    let out = list(bytes);
    assert_eq!(out[0].0, "mimetype");
    assert_eq!(out[0].1, METHOD_STORED);
}

#[test]
fn odt_keeps_mimetype_first_and_stored() {
    let odt = make_odt(true, ODT_MIMETYPE);
    let r = done(
        optimise(
            &odt,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.verification.ok, "{}", r.verification);
    assert!(r.bytes.len() < odt.len());
    assert_mimetype_first_stored(&r.bytes);
    assert_eq!(detect(&r.bytes).unwrap(), Some(OfficeKind::Odt));
    assert_eq!(entry(&list(&r.bytes), "mimetype").2, ODT_MIMETYPE);
    assert_eq!(entry(&list(&r.bytes), "content.xml").1, METHOD_DEFLATE);
    assert_eq!(r.media_recompressed, 1);

    // A sloppy input with mimetype second is repaired, and verify accepts
    // the move as the one permitted reordering.
    let sloppy = make_odt(false, ODT_MIMETYPE);
    assert_ne!(&sloppy[30..38], b"mimetype");
    let r = done(
        optimise(
            &sloppy,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert_mimetype_first_stored(&r.bytes);
    assert!(verify(&sloppy, &r.bytes, None).ok);
}

#[test]
fn epub_keeps_mimetype_first_and_resolves_manifest() {
    let epub = make_epub();
    let r = done(
        optimise(
            &epub,
            &fit(epub.len() as u64 / 2, false),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert!(r.verification.ok, "{}", r.verification);
    assert_mimetype_first_stored(&r.bytes);
    assert_eq!(detect(&r.bytes).unwrap(), Some(OfficeKind::Epub));
    let refs = r
        .verification
        .checks
        .iter()
        .find(|c| c.name == "references_resolve")
        .unwrap();
    assert!(
        refs.detail.starts_with("4 internal references"),
        "{}",
        refs.detail
    );
}

// ---------------------------------------------------------------------------
// Document details.

#[test]
fn keep_document_details_false_blanks_author_and_title_only() {
    let docx = make_ooxml(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
        "word",
        "document.xml",
        false,
    );
    let opts = OfficeOptions {
        keep_document_details: false,
        ..smaller(Mode::SmallerKeepQuality)
    };
    let r = done(optimise(&docx, &opts, &TestRecompressor::images_only(), &never).unwrap());
    assert!(r.verification.ok, "{}", r.verification);
    let core = String::from_utf8(entry(&list(&r.bytes), "docProps/core.xml").2.clone()).unwrap();
    assert!(!core.contains("Grace Hopper"), "{core}");
    assert!(!core.contains("Budget 2026"), "{core}");
    assert!(core.contains("<dc:creator></dc:creator>"), "{core}");
    assert!(core.contains("<cp:revision>2</cp:revision>"), "{core}");
    assert!(core.contains("2026-01-02T03:04:05Z"), "{core}");

    // Default keeps it byte for byte.
    let r = done(
        optimise(
            &docx,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    );
    assert_eq!(
        entry(&list(&r.bytes), "docProps/core.xml").2,
        CORE_XML.as_bytes()
    );

    // ODF meta.xml gets the same treatment.
    let odt = make_odt(true, ODT_MIMETYPE);
    let r = done(optimise(&odt, &opts, &TestRecompressor::images_only(), &never).unwrap());
    let meta = String::from_utf8(entry(&list(&r.bytes), "meta.xml").2.clone()).unwrap();
    assert!(!meta.contains("Ada Lovelace"), "{meta}");
    assert!(
        meta.contains("<meta:editing-cycles>12</meta:editing-cycles>"),
        "{meta}"
    );
}

// ---------------------------------------------------------------------------
// Verification catches broken output.

/// Rewrite `bytes` with `name`'s contents replaced by `replacement` (or the
/// entry dropped when `None`), keeping everything else and the order.
fn rewrite(bytes: &[u8], name: &str, replacement: Option<&[u8]>) -> Vec<u8> {
    let mut zw = ZipWriter::new(Vec::new(), None);
    for (n, _, data) in list(bytes) {
        let data: &[u8] = if n == name {
            match replacement {
                Some(r) => r,
                None => continue,
            }
        } else {
            &data
        };
        zw.write_entry(
            &n,
            METHOD_STORED,
            crc32fast::hash(data),
            data.len() as u64,
            data,
            false,
        )
        .unwrap();
    }
    zw.finish().unwrap()
}

fn failing(report: &cia_office::VerifyReport) -> Vec<&'static str> {
    report.failures().map(|c| c.name).collect()
}

#[test]
fn verify_fails_on_corrupted_xml() {
    let good = done(
        optimise(
            REPORT,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    )
    .bytes;
    assert!(verify(REPORT, &good, None).ok);

    let broken = rewrite(
        &good,
        "word/document.xml",
        Some(b"<?xml version=\"1.0\"?><w:document><w:body><w:p>unclosed</w:body>"),
    );
    let report = verify(REPORT, &broken, None);
    assert!(!report.ok);
    assert_eq!(failing(&report), vec!["xml_parses", "only_media_changed"]);
    assert!(report
        .failures()
        .next()
        .unwrap()
        .detail
        .contains("word/document.xml"));
}

#[test]
fn verify_fails_on_missing_target_bad_crc_wrong_image_and_size() {
    let good = done(
        optimise(
            REPORT,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    )
    .bytes;

    // A relationship target that no longer exists.
    let missing = rewrite(&good, "word/media/image9.png", None);
    let report = verify(REPORT, &missing, None);
    assert_eq!(failing(&report), vec!["entry_order", "references_resolve"]);
    assert!(
        report
            .failures()
            .nth(1)
            .unwrap()
            .detail
            .contains("word/media/image9.png"),
        "{report}"
    );

    // A flipped byte inside a stored entry: the CRC check catches it.
    let mut flipped = good.clone();
    let pos = good.len() / 2;
    flipped[pos] ^= 0xFF;
    let report = verify(REPORT, &flipped, None);
    assert!(!report.ok);
    assert_eq!(failing(&report), vec!["reads"], "{report}");

    // A "JPEG" that is really a PNG does not pass as a replaced image.
    let png = test_png();
    let wrong = rewrite(&good, "word/media/image0.jpeg", Some(&png));
    let report = verify(REPORT, &wrong, None);
    assert_eq!(failing(&report), vec!["replaced_images_decode"], "{report}");

    // Over the limit.
    let report = verify(REPORT, &good, Some(good.len() as u64 - 1));
    assert_eq!(failing(&report), vec!["size"]);
    assert!(verify(REPORT, &good, Some(good.len() as u64)).ok);

    // Not even a zip.
    let report = verify(REPORT, b"garbage", None);
    assert_eq!(failing(&report), vec!["reads"]);
}

#[test]
fn verify_fails_when_mimetype_is_not_first_or_compressed() {
    let odt = make_odt(true, ODT_MIMETYPE);
    let good = done(
        optimise(
            &odt,
            &smaller(Mode::SmallerKeepQuality),
            &TestRecompressor::images_only(),
            &never,
        )
        .unwrap(),
    )
    .bytes;

    // Same entries, but mimetype deflated.
    let mut zw = ZipWriter::new(Vec::new(), None);
    for (n, _, data) in list(&good) {
        let (method, stored): (u16, Vec<u8>) = if n == "mimetype" {
            cia_archive::zip_compress(
                &data,
                &cia_archive::ArchiveOptions {
                    zopfli_max_bytes: 0,
                    ..Default::default()
                },
            )
        } else {
            (METHOD_STORED, data.clone())
        };
        let method = if n == "mimetype" {
            METHOD_DEFLATE
        } else {
            method
        };
        let stored = if n == "mimetype" && method == METHOD_DEFLATE && stored == data {
            // zip_compress stored it (tiny entry); force a real deflate stream.
            let mut enc =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(9));
            std::io::Write::write_all(&mut enc, &data).unwrap();
            enc.finish().unwrap()
        } else {
            stored
        };
        zw.write_entry(
            &n,
            method,
            crc32fast::hash(&data),
            data.len() as u64,
            &stored,
            false,
        )
        .unwrap();
    }
    let deflated = zw.finish().unwrap();
    let report = verify(&odt, &deflated, None);
    assert_eq!(failing(&report), vec!["mimetype_first_stored"], "{report}");
}

// ---------------------------------------------------------------------------
// Cancellation and recompressor errors.

#[test]
fn cancel_and_media_errors_propagate() {
    let cancelled = || true;
    let err = optimise(
        REPORT,
        &smaller(Mode::SmallerKeepQuality),
        &TestRecompressor::images_only(),
        &cancelled,
    )
    .unwrap_err();
    assert_eq!(err, OfficeError::Cancelled);

    struct Failing;
    impl MediaRecompressor for Failing {
        fn recompress_image(
            &self,
            _: &[u8],
            _: ImageFormat,
            _: Option<u64>,
            _: Mode,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Recompressed>, String> {
            Err("encoder exploded".into())
        }
        fn recompress_video(
            &self,
            _: &[u8],
            _: &str,
            _: Option<u64>,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Recompressed>, String> {
            Ok(None)
        }
        fn recompress_audio(
            &self,
            _: &[u8],
            _: &str,
            _: Option<u64>,
            _: &dyn Fn() -> bool,
        ) -> Result<Option<Recompressed>, String> {
            Ok(None)
        }
    }
    let err = optimise(REPORT, &smaller(Mode::SmallerKeepQuality), &Failing, &never).unwrap_err();
    assert_eq!(
        err,
        OfficeError::Media {
            entry: "word/media/image0.jpeg".into(),
            message: "encoder exploded".into(),
        }
    );
}

#[test]
fn damaged_zip_is_corrupt_not_a_panic() {
    let mut truncated = REPORT[..REPORT.len() / 2].to_vec();
    truncated.extend_from_slice(&REPORT[REPORT.len() - 22..]);
    match detect(&truncated) {
        Ok(None) | Err(OfficeError::Corrupt(_)) | Err(OfficeError::Unsupported(_)) => {}
        other => panic!("{other:?}"),
    }
    let bytes = Cursor::new(REPORT).into_inner();
    assert!(optimise(
        &bytes[..1000],
        &smaller(Mode::SmallerKeepQuality),
        &TestRecompressor::images_only(),
        &never
    )
    .is_err());
}
