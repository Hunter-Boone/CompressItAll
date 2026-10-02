//! Step 5 of the PDF planner, and the honesty rule of DESIGN.md 3.11: the
//! bytes we hand back must reload, keep every page, keep every content
//! stream byte for byte, carry images that decode, and fit the limit.

use std::fmt;

use lopdf::{Document, Object};

use crate::doc::{entry_i64, filters, is_image_xobject, load, plain_bytes};
use crate::encrypted;
use crate::images::decode_jpeg_raw;
use crate::PdfError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub ok: bool,
    pub checks: Vec<VerifyCheck>,
    pub page_count: u32,
    pub output_bytes: u64,
}

impl VerifyReport {
    fn push(&mut self, name: &'static str, passed: bool, detail: impl Into<String>) {
        self.checks.push(VerifyCheck {
            name,
            passed,
            detail: detail.into(),
        });
        self.ok &= passed;
    }

    pub fn failures(&self) -> impl Iterator<Item = &VerifyCheck> {
        self.checks.iter().filter(|c| !c.passed)
    }
}

impl fmt::Display for VerifyReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.checks {
            writeln!(
                f,
                "[{}] {}: {}",
                if c.passed { "ok" } else { "FAIL" },
                c.name,
                c.detail
            )?;
        }
        Ok(())
    }
}

/// Check `output` against `original`. Fails with [`PdfError::Damaged`] only
/// when the *original* cannot be read; every problem with the output is a
/// failed check in the report.
pub fn verify(
    original: &[u8],
    output: &[u8],
    hard_bytes: Option<u64>,
) -> Result<VerifyReport, PdfError> {
    let orig = load(original)?;
    let mut report = VerifyReport {
        ok: true,
        checks: Vec::new(),
        page_count: 0,
        output_bytes: output.len() as u64,
    };

    let out_doc = match load(output) {
        Ok(d) => d,
        Err(e) => {
            report.push("loads", false, format!("output does not load: {e}"));
            return Ok(report);
        }
    };
    report.push(
        "loads",
        true,
        format!("lopdf reads it (PDF {})", out_doc.version),
    );

    let enc = encrypted::declares_encrypt(output) || out_doc.is_encrypted();
    report.push(
        "not_encrypted",
        !enc,
        if enc {
            "output declares /Encrypt"
        } else {
            "no /Encrypt"
        }
        .to_string(),
    );

    let orig_pages = orig.get_pages();
    let out_pages = out_doc.get_pages();
    report.page_count = out_pages.len() as u32;
    report.push(
        "page_count",
        orig_pages.len() == out_pages.len(),
        format!(
            "{} pages before, {} after",
            orig_pages.len(),
            out_pages.len()
        ),
    );

    // Content streams: every one decodes, and the per-page concatenation
    // matches the original where the original itself is readable.
    let mut decode_failures = Vec::new();
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    for (num, out_id) in &out_pages {
        let out_content = match page_content(&out_doc, *out_id) {
            Ok(c) => c,
            Err(e) => {
                decode_failures.push(format!("page {num}: {e}"));
                continue;
            }
        };
        if let Some(orig_id) = orig_pages.get(num) {
            if let Ok(orig_content) = page_content(&orig, *orig_id) {
                compared += 1;
                if orig_content != out_content {
                    mismatches.push(format!("page {num}"));
                }
            }
        }
    }
    report.push(
        "content_streams_decode",
        decode_failures.is_empty(),
        if decode_failures.is_empty() {
            format!("all {} pages' content streams decode", out_pages.len())
        } else {
            decode_failures.join("; ")
        },
    );
    report.push(
        "content_streams_unchanged",
        mismatches.is_empty(),
        if mismatches.is_empty() {
            format!("{compared} pages compared byte for byte")
        } else {
            format!("content differs on {}", mismatches.join(", "))
        },
    );

    // Images: every DCT image decodes, every Flate/raw image inflates to at
    // least the size its dictionary implies.
    let (checked, unchecked, failures) = check_images(&out_doc);
    report.push(
        "images_decode",
        failures.is_empty(),
        if failures.is_empty() {
            format!("{checked} images decode, {unchecked} with other codecs left as is")
        } else {
            failures.join("; ")
        },
    );

    match hard_bytes {
        Some(limit) => report.push(
            "size",
            output.len() as u64 <= limit,
            format!("{} bytes, limit {limit}", output.len()),
        ),
        None => report.push("size", true, format!("{} bytes, no limit", output.len())),
    }

    Ok(report)
}

/// Decoded content of a page, all streams joined with a newline (the
/// separator the PDF spec implies between concatenated content streams).
fn page_content(doc: &Document, page_id: lopdf::ObjectId) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for id in doc.get_page_contents(page_id) {
        let stream = match doc.get_object(id) {
            Ok(Object::Stream(s)) => s,
            Ok(_) => return Err(format!("content {id:?} is not a stream")),
            Err(e) => return Err(format!("content {id:?}: {e}")),
        };
        let f = filters(doc, stream);
        let bytes = if f.is_empty() || f == [b"FlateDecode".to_vec()] {
            plain_bytes(doc, stream)?
        } else {
            stream
                .decompressed_content()
                .map_err(|e| format!("content {id:?}: {e}"))?
        };
        out.extend_from_slice(&bytes);
        out.push(b'\n');
    }
    Ok(out)
}

fn check_images(doc: &Document) -> (usize, usize, Vec<String>) {
    let mut checked = 0;
    let mut unchecked = 0;
    let mut failures = Vec::new();
    for (id, obj) in &doc.objects {
        let Object::Stream(s) = obj else { continue };
        if !is_image_xobject(&s.dict) {
            continue;
        }
        let f = filters(doc, s);
        let names: Vec<&[u8]> = f.iter().map(|n| n.as_slice()).collect();
        match names.as_slice() {
            [b"DCTDecode"] => match decode_jpeg_raw(&s.content) {
                Ok((w, h, _, _)) => {
                    let dw = entry_i64(doc, &s.dict, b"Width").unwrap_or(-1);
                    let dh = entry_i64(doc, &s.dict, b"Height").unwrap_or(-1);
                    if dw != w as i64 || dh != h as i64 {
                        failures.push(format!(
                            "image {id:?}: JPEG is {w}x{h}, dictionary says {dw}x{dh}"
                        ));
                    } else {
                        checked += 1;
                    }
                }
                Err(e) => failures.push(format!("image {id:?}: {e}")),
            },
            [] | [b"FlateDecode"] => match plain_bytes(doc, s) {
                Ok(bytes) => {
                    let w = entry_i64(doc, &s.dict, b"Width").unwrap_or(0).max(0) as usize;
                    let h = entry_i64(doc, &s.dict, b"Height").unwrap_or(0).max(0) as usize;
                    let bpc = entry_i64(doc, &s.dict, b"BitsPerComponent")
                        .unwrap_or(8)
                        .max(0) as usize;
                    // Without knowing the component count we can only bound
                    // from below with one component per sample.
                    let min_len = (w * bpc).div_ceil(8) * h;
                    if bytes.len() < min_len {
                        failures.push(format!(
                            "image {id:?}: {} bytes, at least {min_len} expected",
                            bytes.len()
                        ));
                    } else {
                        checked += 1;
                    }
                }
                Err(e) => failures.push(format!("image {id:?}: {e}")),
            },
            _ => unchecked += 1,
        }
    }
    (checked, unchecked, failures)
}
