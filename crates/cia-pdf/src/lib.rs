#![forbid(unsafe_code)]
//! Smidge PDF optimiser (DESIGN.md 3.7). Bytes in, bytes out.
//!
//! The planner refuses encrypted and damaged files, always runs a lossless
//! structural pass, and then walks one global `(quality, pixel cap)` ladder
//! over every embedded image until the file fits. JPEG encoding is supplied
//! by the caller through [`JpegEncoder`] so this crate stays free of C codecs.

mod codec;
mod doc;
mod encrypted;
mod images;
mod lossless;
mod resize;
mod verify;

use std::fmt;

pub use cia_core::SmallerLevel;
pub use verify::{verify, VerifyCheck, VerifyReport};

/// Supplies JPEG encoding (mozjpeg in the product, anything in tests).
pub trait JpegEncoder {
    fn encode_rgb(&self, rgb: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String>;
    fn encode_gray(&self, gray: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfOptions {
    /// Keep XMP metadata streams. Thumbnails and `/PieceInfo` always go.
    pub keep_document_details: bool,
    /// Fit mode: the output must be at most this many bytes.
    pub budget_bytes: Option<u64>,
    /// Smaller mode (no budget). `KeepQuality` stops after the lossless pass
    /// unless quality-85 images save more than 10 percent; `Smallest` takes
    /// the `(65, 3000)` ladder step when it helps.
    pub smaller_mode: Option<SmallerLevel>,
}

/// One saved candidate during the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfAttempt {
    /// 0 is the lossless pass; 1..=7 index the ladder.
    pub step: usize,
    pub quality: Option<u8>,
    pub cap: Option<u32>,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfResult {
    pub bytes: Vec<u8>,
    /// Ladder step that produced `bytes` (0 = lossless only).
    pub step: usize,
    pub quality: Option<u8>,
    pub pixel_cap: Option<u32>,
    pub images_recompressed: usize,
    pub images_skipped: usize,
    /// Size after the lossless pass alone.
    pub lossless_bytes: u64,
    pub attempts: Vec<PdfAttempt>,
    pub verification: VerifyReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfOutcome {
    Done(PdfResult),
    /// Nothing worth writing: the original already fits (or there is no
    /// budget) and we could not make it at least 5 percent smaller.
    KeptOriginal {
        reason: String,
    },
    /// Even the bottom of the ladder does not fit.
    Refused {
        smallest_bytes: u64,
    },
    Encrypted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfInfo {
    pub page_count: u32,
    pub encrypted: bool,
    pub image_count: usize,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PdfError {
    #[error("This PDF is damaged: {0}")]
    Damaged(String),
    #[error("cancelled")]
    Cancelled,
    #[error("output failed verification:\n{0}")]
    Verification(VerifyReport),
    #[error("internal error: {0}")]
    Internal(String),
}

/// The global search ladder: `(quality, long-edge cap)`.
pub const LADDER: [(u8, Option<u32>); 7] = [
    (85, None),
    (75, Some(4000)),
    (65, Some(3000)),
    (55, Some(2400)),
    (50, Some(2000)),
    (45, Some(1600)),
    (45, Some(1200)),
];

/// Ladder index used for Smaller/Smallest (`(65, 3000)`).
const SMALLEST_STEP: usize = 2;

/// Cheap facts about a PDF: enough for the type row before any work starts.
pub fn inspect(bytes: &[u8]) -> Result<PdfInfo, PdfError> {
    let declared = encrypted::declares_encrypt(bytes);
    match doc::load(bytes) {
        Ok(d) => {
            let encrypted = declared || d.is_encrypted() || d.was_encrypted();
            Ok(PdfInfo {
                page_count: d.get_pages().len() as u32,
                encrypted,
                image_count: images::collect_images(&d).total(),
                version: d.version.clone(),
            })
        }
        Err(_) if declared => Ok(PdfInfo {
            page_count: 0,
            encrypted: true,
            image_count: 0,
            version: header_version(bytes),
        }),
        Err(e) => Err(e),
    }
}

fn header_version(bytes: &[u8]) -> String {
    bytes
        .get(5..8)
        .filter(|_| bytes.starts_with(b"%PDF-"))
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_default()
}

struct Search<'a> {
    original: &'a [u8],
    lossless_doc: lopdf::Document,
    targets: Vec<images::ImageTarget>,
    ineligible: usize,
    cache: images::DecodeCache,
    enc: &'a dyn JpegEncoder,
    cancel: &'a dyn Fn() -> bool,
    attempts: Vec<PdfAttempt>,
}

struct Candidate {
    bytes: Vec<u8>,
    step: usize,
    quality: Option<u8>,
    cap: Option<u32>,
    recompressed: usize,
    skipped: usize,
}

impl Search<'_> {
    fn run_step(&mut self, step: usize) -> Result<Candidate, PdfError> {
        if (self.cancel)() {
            return Err(PdfError::Cancelled);
        }
        let (quality, cap) = LADDER[step - 1];
        let mut d = self.lossless_doc.clone();
        let stats = images::image_pass(
            &mut d,
            &self.targets,
            quality,
            cap,
            self.enc,
            &mut self.cache,
            self.cancel,
        )?;
        // Identical images that were re-encoded identically fold together.
        lossless::dedupe_streams(&mut d);
        let bytes = doc::save(&mut d)?;
        self.attempts.push(PdfAttempt {
            step,
            quality: Some(quality),
            cap,
            bytes: bytes.len() as u64,
        });
        Ok(Candidate {
            bytes,
            step,
            quality: Some(quality),
            cap,
            recompressed: stats.recompressed,
            skipped: stats.skipped + self.ineligible,
        })
    }

    fn finish(
        self,
        c: Candidate,
        lossless_bytes: u64,
        limit: Option<u64>,
    ) -> Result<PdfOutcome, PdfError> {
        let verification = verify(self.original, &c.bytes, limit)?;
        if !verification.ok {
            return Err(PdfError::Verification(verification));
        }
        Ok(PdfOutcome::Done(PdfResult {
            bytes: c.bytes,
            step: c.step,
            quality: c.quality,
            pixel_cap: c.cap,
            images_recompressed: c.recompressed,
            images_skipped: c.skipped,
            lossless_bytes,
            attempts: self.attempts,
            verification,
        }))
    }
}

/// Run the planner. `cancel` is polled between images and between ladder
/// steps.
pub fn optimise(
    bytes: &[u8],
    opts: &PdfOptions,
    enc: &dyn JpegEncoder,
    cancel: &dyn Fn() -> bool,
) -> Result<PdfOutcome, PdfError> {
    if encrypted::declares_encrypt(bytes) {
        return Ok(PdfOutcome::Encrypted);
    }
    let mut d = doc::load(bytes)?;
    if d.is_encrypted() || d.was_encrypted() {
        return Ok(PdfOutcome::Encrypted);
    }
    if cancel() {
        return Err(PdfError::Cancelled);
    }

    let stats = lossless::lossless_pass(&mut d, opts.keep_document_details);
    log::debug!("lossless pass: {stats:?}");
    let lossless_bytes = doc::save(&mut d.clone())?;
    let lossless_len = lossless_bytes.len() as u64;

    let collected = images::collect_images(&d);
    let mut search = Search {
        original: bytes,
        lossless_doc: d,
        ineligible: collected.ineligible,
        targets: collected.targets,
        cache: images::DecodeCache::default(),
        enc,
        cancel,
        attempts: vec![PdfAttempt {
            step: 0,
            quality: None,
            cap: None,
            bytes: lossless_len,
        }],
    };
    let lossless_candidate = |bytes: Vec<u8>| Candidate {
        bytes,
        step: 0,
        quality: None,
        cap: None,
        recompressed: 0,
        skipped: search.targets.len() + search.ineligible,
    };
    let original_len = bytes.len() as u64;
    let no_images = search.targets.is_empty();

    match opts.budget_bytes {
        Some(budget) if lossless_len > budget => {
            // Walk the ladder until something fits.
            let mut smallest = lossless_len;
            for step in 1..=LADDER.len() {
                let c = search.run_step(step)?;
                let len = c.bytes.len() as u64;
                if len <= budget {
                    return search.finish(c, lossless_len, Some(budget));
                }
                smallest = smallest.min(len);
                if no_images {
                    break;
                }
            }
            Ok(PdfOutcome::Refused {
                smallest_bytes: smallest,
            })
        }
        budget => {
            // Lossless already fits (or there is no budget). Take the image
            // pass only when it is clearly worth the quality trade.
            let smaller = opts.smaller_mode.unwrap_or(SmallerLevel::KeepQuality);
            let mut best = lossless_candidate(lossless_bytes);
            if !no_images {
                let (step, min_saving_pct) = match smaller {
                    SmallerLevel::KeepQuality => (1, 10),
                    SmallerLevel::Smallest => (SMALLEST_STEP + 1, 0),
                };
                let c = search.run_step(step)?;
                let saving_ok =
                    (c.bytes.len() as u64) * 100 < lossless_len * (100 - min_saving_pct);
                if saving_ok {
                    best = c;
                }
            }
            let original_fits = budget.is_none_or(|b| original_len <= b);
            if original_fits && (best.bytes.len() as u64) * 100 > original_len * 95 {
                return Ok(PdfOutcome::KeptOriginal {
                    reason: format!(
                        "already as small as it gets ({} bytes; best attempt {} bytes)",
                        original_len,
                        best.bytes.len()
                    ),
                });
            }
            search.finish(best, lossless_len, budget)
        }
    }
}

impl fmt::Display for PdfOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfOutcome::Done(r) => write!(
                f,
                "done: {} bytes at step {} (q {:?}, cap {:?}), {} images recompressed, {} skipped",
                r.bytes.len(),
                r.step,
                r.quality,
                r.pixel_cap,
                r.images_recompressed,
                r.images_skipped
            ),
            PdfOutcome::KeptOriginal { reason } => write!(f, "kept original: {reason}"),
            PdfOutcome::Refused { smallest_bytes } => {
                write!(f, "refused: smallest reached {smallest_bytes} bytes")
            }
            PdfOutcome::Encrypted => write!(f, "encrypted"),
        }
    }
}
