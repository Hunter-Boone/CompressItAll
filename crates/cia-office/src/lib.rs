#![forbid(unsafe_code)]
//! Smidge Office document planner (DESIGN.md 3.8). Bytes in, bytes out.
//!
//! docx, docm, pptx, pptm, xlsx, xlsm, odt, odp, ods and epub are ZIP
//! packages. This crate recompresses the embedded media and the ZIP itself
//! and never edits document XML. The media codecs live elsewhere: the caller
//! supplies a [`MediaRecompressor`] (cia-image, cia-video, cia-audio in the
//! product; the `image` crate in tests) and this crate decides which entries
//! to hand it, with what budget (3.9.1 water-filling via
//! `cia_core::allocation`), and rebuilds and verifies the package.
//!
//! # Flow
//!
//! 1. [`detect`] says which kind a byte slice is; OLE compound files (the
//!    wrapper around password-protected OOXML) are refused with
//!    [`OfficeError::Encrypted`].
//! 2. [`inspect`] lists entries and media and estimates the fixed bytes.
//! 3. [`optimise`] does the work and verifies its own output with
//!    [`verify`] before returning [`OfficeOutcome::Done`].
//!
//! # Rules kept (3.8)
//!
//! Images keep their format and extension so `[Content_Types].xml` and the
//! relationship parts never change. Video and audio keep their container.
//! Non-media entries are measured at deflate level 9 (zopfli under 1 MB) and
//! treated as fixed; the remaining budget is water-filled across media.
//! Output keeps the original entry order; ODF and EPUB get `mimetype` first
//! and stored. Already-compressed media is stored when deflate saves under
//! one percent. If even the quality floor does not fit, the planner refuses
//! with the smallest size it could reach.

mod detect;
mod metadata;
mod package;
mod verify;
mod write;
mod xml;

use std::fmt;

use cia_core::allocation::{allocate, Allocation, ItemSizes};
pub use cia_core::QualityLabel;
pub use detect::detect;
pub use verify::{verify, VerifyCheck, VerifyReport};

use package::Package;
use write::Part;

/// Which package this is (DESIGN.md 3.3, OfficeDoc row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OfficeKind {
    Docx,
    Docm,
    Pptx,
    Pptm,
    Xlsx,
    Xlsm,
    Odt,
    Odp,
    Ods,
    Epub,
}

impl OfficeKind {
    /// File extension without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            OfficeKind::Docx => "docx",
            OfficeKind::Docm => "docm",
            OfficeKind::Pptx => "pptx",
            OfficeKind::Pptm => "pptm",
            OfficeKind::Xlsx => "xlsx",
            OfficeKind::Xlsm => "xlsm",
            OfficeKind::Odt => "odt",
            OfficeKind::Odp => "odp",
            OfficeKind::Ods => "ods",
            OfficeKind::Epub => "epub",
        }
    }

    /// Short user-facing name for the type row.
    pub fn description(self) -> &'static str {
        match self {
            OfficeKind::Docx | OfficeKind::Docm => "Word document",
            OfficeKind::Pptx | OfficeKind::Pptm => "PowerPoint presentation",
            OfficeKind::Xlsx | OfficeKind::Xlsm => "Excel workbook",
            OfficeKind::Odt => "OpenDocument text",
            OfficeKind::Odp => "OpenDocument presentation",
            OfficeKind::Ods => "OpenDocument spreadsheet",
            OfficeKind::Epub => "EPUB book",
        }
    }

    /// Office Open XML (`[Content_Types].xml`, `_rels`).
    pub fn is_ooxml(self) -> bool {
        matches!(
            self,
            OfficeKind::Docx
                | OfficeKind::Docm
                | OfficeKind::Pptx
                | OfficeKind::Pptm
                | OfficeKind::Xlsx
                | OfficeKind::Xlsm
        )
    }

    /// OpenDocument (`mimetype`, `META-INF/manifest.xml`, `Pictures/`).
    pub fn is_odf(self) -> bool {
        matches!(self, OfficeKind::Odt | OfficeKind::Odp | OfficeKind::Ods)
    }

    /// Formats whose spec wants a stored `mimetype` entry first.
    pub fn has_mimetype(self) -> bool {
        self.is_odf() || self == OfficeKind::Epub
    }
}

impl fmt::Display for OfficeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// Image formats this planner recompresses (always to the same format).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

impl ImageFormat {
    /// True when `bytes` start with this format's signature.
    pub fn matches(self, bytes: &[u8]) -> bool {
        match self {
            ImageFormat::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
            ImageFormat::Png => {
                bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
            }
        }
    }
}

/// What a media entry is, decided from its location, extension and magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// A JPEG or PNG whose bytes match its extension.
    Image(ImageFormat),
    Video,
    Audio,
    /// Left exactly as it is: EMF, WMF, SVG, GIF, TIFF, mismatched
    /// extensions, anything unknown.
    Other,
}

/// How hard to push (DESIGN.md 3.1 goals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Fit a byte budget; `OfficeOptions::budget_bytes` must be set.
    Fit,
    /// No budget; take savings that keep quality (lossless and near-lossless).
    SmallerKeepQuality,
    /// No budget; take the planner's smallest acceptable quality.
    SmallerSmallest,
}

/// A smaller encoding of one media entry, same format as the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recompressed {
    pub bytes: Vec<u8>,
    /// The quality the encoder predicts for this encoding.
    pub quality_label: QualityLabel,
}

/// Media codecs supplied by the caller. Every method returns `Ok(None)` to
/// mean "leave the original as it is"; `Err` aborts the document with
/// [`OfficeError::Media`].
///
/// Contract this crate relies on:
///
/// - `recompress_image` with `budget = None` and [`Mode::SmallerKeepQuality`]
///   is the lossless/optimised size (`L` in 3.9.1); with
///   [`Mode::SmallerSmallest`] it is the quality floor at full size (`F`);
///   with [`Mode::Fit`] and `budget = Some(b)` it is the best encoding that
///   fits in `b` bytes, or `None` if nothing under `b` is acceptable.
/// - `recompress_video` / `recompress_audio` with `budget = None` keep
///   quality; with `Some(b)` they fit `b`. They have no floor probe, so the
///   planner asks them for whatever the images leave over (see
///   [`optimise`]).
/// - Output must be the same format as the input (jpeg to jpeg, png to png,
///   mp4 to mp4), because the package's content types and relationships are
///   never rewritten.
pub trait MediaRecompressor {
    fn recompress_image(
        &self,
        bytes: &[u8],
        format: ImageFormat,
        budget: Option<u64>,
        mode: Mode,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String>;

    fn recompress_video(
        &self,
        bytes: &[u8],
        ext: &str,
        budget: Option<u64>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String>;

    fn recompress_audio(
        &self,
        bytes: &[u8],
        ext: &str,
        budget: Option<u64>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Option<Recompressed>, String>;
}

/// One ZIP entry as listed by [`inspect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub name: String,
    /// Uncompressed size.
    pub size: u64,
    pub is_dir: bool,
}

/// One media entry with its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEntry {
    pub name: String,
    pub bytes: Vec<u8>,
    pub kind: MediaKind,
}

/// Cheap facts about a package: enough for the type row and a prediction
/// before any media work starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeInfo {
    pub kind: OfficeKind,
    pub entries: Vec<EntryInfo>,
    pub media: Vec<MediaEntry>,
    /// Container overhead plus every non-media entry at deflate level 9
    /// (without zopfli, so this is a slight over-estimate of what
    /// [`optimise`] achieves). The media budget is the limit minus this.
    pub fixed_bytes_estimate: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeOptions {
    /// Fit mode: the output must be at most this many bytes.
    pub budget_bytes: Option<u64>,
    /// [`Mode::Fit`] without a budget behaves as [`Mode::SmallerKeepQuality`].
    pub mode: Mode,
    /// When false, author/title style fields in `docProps/core.xml` and
    /// ODF `meta.xml` are blanked (the files stay). EPUB metadata is the
    /// book's own and is never touched.
    pub keep_document_details: bool,
    /// Whether the host can recompress video (desktop with FFmpeg). When
    /// false, videos are left as they are and the notes say so.
    pub host_can_video: bool,
}

impl Default for OfficeOptions {
    fn default() -> Self {
        Self {
            budget_bytes: None,
            mode: Mode::SmallerKeepQuality,
            keep_document_details: true,
            host_can_video: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficeResult {
    pub bytes: Vec<u8>,
    /// Media entries whose bytes changed.
    pub media_recompressed: usize,
    /// Media entries written unchanged (including EMF/SVG and the like).
    pub media_left: usize,
    /// The lowest label among the recompressed media; `Great` when every
    /// change was lossless or there was no media.
    pub predicted_quality: QualityLabel,
    /// User-facing remarks, e.g. `"1 video left as is"`.
    pub notes: Vec<String>,
    pub verification: VerifyReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfficeOutcome {
    Done(OfficeResult),
    /// The original already fits (or there is no budget) and the rebuilt
    /// package is not at least 5 percent smaller.
    KeptOriginal {
        reason: String,
    },
    /// Even every medium at its quality floor does not fit.
    Refused {
        smallest_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OfficeError {
    /// An OLE compound file: password-protected OOXML (or a legacy binary
    /// .doc/.xls/.ppt, which looks the same from the outside).
    #[error("This document is password-protected")]
    Encrypted,
    /// Not a ZIP, or a ZIP without the Office markers.
    #[error("not an Office document")]
    NotOffice,
    #[error("This document is damaged: {0}")]
    Corrupt(String),
    /// A ZIP feature this build cannot read (compression method, size).
    #[error("unsupported package: {0}")]
    Unsupported(String),
    #[error("cancelled")]
    Cancelled,
    /// The caller's recompressor failed on one entry.
    #[error("{entry}: {message}")]
    Media { entry: String, message: String },
    #[error("output failed verification:\n{0}")]
    Verification(VerifyReport),
    #[error("internal error: {0}")]
    Internal(String),
}

/// Keep the original unless the rebuilt package is at least this much
/// smaller (as a fraction of the original), matching 3.4.7 for images.
const MIN_SAVING: f64 = 0.05;

/// Round 2 of 3.9.1 runs when at least this fraction of the budget is left.
const ROUND2_MIN_LEFTOVER: f64 = 0.05;

/// List entries and media and estimate the fixed bytes.
pub fn inspect(bytes: &[u8]) -> Result<OfficeInfo, OfficeError> {
    let pkg = package::read(bytes)?;
    let quick = write::quick_options();
    let mut fixed = write::overhead_bytes(&pkg.entries);
    for (i, e) in pkg.entries.iter().enumerate() {
        if e.is_dir || pkg.media_kind(i).is_some() {
            continue;
        }
        fixed += if pkg.kind.has_mimetype() && e.name == "mimetype" {
            e.bytes.len() as u64
        } else {
            write::compress(&e.bytes, &quick).stored_len()
        };
    }
    Ok(OfficeInfo {
        kind: pkg.kind,
        entries: pkg
            .entries
            .iter()
            .map(|e| EntryInfo {
                name: e.name.clone(),
                size: e.bytes.len() as u64,
                is_dir: e.is_dir,
            })
            .collect(),
        media: pkg
            .media
            .iter()
            .map(|(i, kind)| MediaEntry {
                name: pkg.entries[*i].name.clone(),
                bytes: pkg.entries[*i].bytes.clone(),
                kind: *kind,
            })
            .collect(),
        fixed_bytes_estimate: fixed,
    })
}

/// Rebuild `bytes` under `options`. The returned [`OfficeOutcome::Done`]
/// has already passed [`verify`]; a failed verification is
/// [`OfficeError::Verification`] rather than a result.
pub fn optimise(
    bytes: &[u8],
    options: &OfficeOptions,
    media: &dyn MediaRecompressor,
    cancel: &dyn Fn() -> bool,
) -> Result<OfficeOutcome, OfficeError> {
    let pkg = package::read(bytes)?;
    let planner = Planner::new(&pkg, options, media, cancel)?;
    match (options.mode, options.budget_bytes) {
        (Mode::Fit, Some(budget)) => planner.fit(bytes, budget),
        (Mode::Fit, None) | (Mode::SmallerKeepQuality, _) => {
            planner.smaller(bytes, Mode::SmallerKeepQuality)
        }
        (Mode::SmallerSmallest, _) => planner.smaller(bytes, Mode::SmallerSmallest),
    }
}

/// One media entry the recompressor may change.
struct Item {
    index: usize,
    kind: MediaKind,
    original: u64,
    /// Keep-quality result, when smaller than the original.
    lossless: Option<Recompressed>,
    /// Quality-floor result, when smaller than `lossless`.
    floor: Option<Recompressed>,
    /// What goes into the package; `None` writes the original bytes.
    chosen: Option<Recompressed>,
    /// Budget handed out in round 1 (Fit mode).
    budget: u64,
}

impl Item {
    fn lossless_len(&self) -> u64 {
        self.lossless
            .as_ref()
            .map_or(self.original, |r| r.bytes.len() as u64)
    }

    fn floor_len(&self) -> u64 {
        self.floor
            .as_ref()
            .map_or(self.lossless_len(), |r| r.bytes.len() as u64)
    }

    fn chosen_len(&self) -> u64 {
        self.chosen
            .as_ref()
            .map_or(self.original, |r| r.bytes.len() as u64)
    }

    fn smallest(&self) -> Option<Recompressed> {
        self.floor.clone().or_else(|| self.lossless.clone())
    }
}

struct Planner<'a> {
    pkg: &'a Package,
    media: &'a dyn MediaRecompressor,
    cancel: &'a dyn Fn() -> bool,
    /// Prepared parts for every entry that will not change; `None` for
    /// items and directories.
    parts: Vec<Option<Part>>,
    /// Container overhead plus every prepared part.
    fixed_bytes: u64,
    items: Vec<Item>,
    videos_left: usize,
}

impl<'a> Planner<'a> {
    fn new(
        pkg: &'a Package,
        options: &'a OfficeOptions,
        media: &'a dyn MediaRecompressor,
        cancel: &'a dyn Fn() -> bool,
    ) -> Result<Self, OfficeError> {
        let exact = write::exact_options();
        let quick = write::quick_options();
        let mut parts: Vec<Option<Part>> = Vec::with_capacity(pkg.entries.len());
        let mut fixed_bytes = write::overhead_bytes(&pkg.entries);
        let mut items = Vec::new();
        let mut videos_left = 0usize;
        for (i, e) in pkg.entries.iter().enumerate() {
            if (cancel)() {
                return Err(OfficeError::Cancelled);
            }
            if e.is_dir {
                parts.push(None);
                continue;
            }
            let kind = pkg.media_kind(i);
            let eligible = match kind {
                Some(MediaKind::Image(_)) | Some(MediaKind::Audio) => true,
                Some(MediaKind::Video) => options.host_can_video,
                Some(MediaKind::Other) | None => false,
            };
            if eligible {
                items.push(Item {
                    index: i,
                    kind: kind.expect("eligible entries are media"),
                    original: e.bytes.len() as u64,
                    lossless: None,
                    floor: None,
                    chosen: None,
                    budget: 0,
                });
                parts.push(None);
                continue;
            }
            if kind == Some(MediaKind::Video) {
                videos_left += 1;
            }
            let part = if pkg.kind.has_mimetype() && e.name == "mimetype" {
                write::stored(&e.bytes)
            } else if kind.is_some() {
                // Media we are not touching: stored unless deflate pays.
                write::compress(&e.bytes, &quick)
            } else if !options.keep_document_details {
                match metadata::strip_details(pkg.kind, &e.name, &e.bytes) {
                    Some(stripped) => write::compress(&stripped, &exact),
                    None => write::compress(&e.bytes, &exact),
                }
            } else {
                write::compress(&e.bytes, &exact)
            };
            fixed_bytes += part.stored_len();
            parts.push(Some(part));
        }
        Ok(Self {
            pkg,
            media,
            cancel,
            parts,
            fixed_bytes,
            items,
            videos_left,
        })
    }

    fn check_cancel(&self) -> Result<(), OfficeError> {
        if (self.cancel)() {
            Err(OfficeError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Call the recompressor for item `k`.
    fn encode(
        &self,
        k: usize,
        budget: Option<u64>,
        mode: Mode,
    ) -> Result<Option<Recompressed>, OfficeError> {
        self.check_cancel()?;
        let item = &self.items[k];
        let entry = &self.pkg.entries[item.index];
        let ext = package::extension(&entry.name);
        let r = match item.kind {
            MediaKind::Image(fmt) => {
                self.media
                    .recompress_image(&entry.bytes, fmt, budget, mode, self.cancel)
            }
            MediaKind::Video => {
                self.media
                    .recompress_video(&entry.bytes, &ext, budget, self.cancel)
            }
            MediaKind::Audio => {
                self.media
                    .recompress_audio(&entry.bytes, &ext, budget, self.cancel)
            }
            MediaKind::Other => Ok(None),
        };
        match r {
            Ok(Some(r)) if r.bytes.is_empty() => Err(OfficeError::Media {
                entry: entry.name.clone(),
                message: "recompressor returned no bytes".into(),
            }),
            Ok(r) => Ok(r),
            Err(message) => Err(OfficeError::Media {
                entry: entry.name.clone(),
                message,
            }),
        }
    }

    /// Pass 0, first half: `L` for every item.
    fn probe_lossless(&mut self) -> Result<(), OfficeError> {
        for k in 0..self.items.len() {
            let r = self.encode(k, None, Mode::SmallerKeepQuality)?;
            let item = &mut self.items[k];
            item.lossless = r.filter(|r| (r.bytes.len() as u64) < item.original);
        }
        Ok(())
    }

    /// Pass 0, second half: `F` for images. Video and audio have no floor
    /// probe; their floor starts at `L` and may drop in [`Planner::fit`].
    fn probe_floor(&mut self) -> Result<(), OfficeError> {
        for k in 0..self.items.len() {
            if !matches!(self.items[k].kind, MediaKind::Image(_)) {
                continue;
            }
            let r = self.encode(k, None, Mode::SmallerSmallest)?;
            let item = &mut self.items[k];
            item.floor = r.filter(|r| (r.bytes.len() as u64) < item.lossless_len());
        }
        Ok(())
    }

    fn allocate(&self, media_budget: u64) -> Allocation {
        let sizes: Vec<ItemSizes> = self
            .items
            .iter()
            .map(|it| ItemSizes {
                id: self.pkg.entries[it.index].name.clone(),
                lossless: it.lossless_len(),
                floor: it.floor_len(),
            })
            .collect();
        allocate(&sizes, media_budget)
    }

    fn sum_floor(&self) -> u64 {
        self.items.iter().map(Item::floor_len).sum()
    }

    fn fit(mut self, original: &[u8], budget: u64) -> Result<OfficeOutcome, OfficeError> {
        self.probe_lossless()?;
        let sum_l: u64 = self.items.iter().map(Item::lossless_len).sum();

        if self.fixed_bytes + sum_l <= budget {
            for it in &mut self.items {
                it.chosen = it.lossless.clone();
            }
        } else {
            self.probe_floor()?;
            let media_budget = budget.saturating_sub(self.fixed_bytes);
            let mut alloc = self.allocate(media_budget);

            // Video and audio have no floor of their own: ask them to fit
            // what the images leave once the images sit at their floor. If
            // they manage it, the second allocation hands any slack back.
            if matches!(alloc, Allocation::TooBig { .. }) {
                let image_floor: u64 = self
                    .items
                    .iter()
                    .filter(|it| matches!(it.kind, MediaKind::Image(_)))
                    .map(Item::floor_len)
                    .sum();
                let av: Vec<usize> = (0..self.items.len())
                    .filter(|&k| !matches!(self.items[k].kind, MediaKind::Image(_)))
                    .collect();
                let av_total: u64 = av.iter().map(|&k| self.items[k].lossless_len()).sum();
                let spare = media_budget.saturating_sub(image_floor);
                if !av.is_empty() && spare > 0 && av_total > 0 {
                    for &k in &av {
                        let share = (u128::from(spare) * u128::from(self.items[k].lossless_len())
                            / u128::from(av_total)) as u64;
                        if share == 0 {
                            continue;
                        }
                        let r = self.encode(k, Some(share), Mode::Fit)?;
                        let item = &mut self.items[k];
                        if let Some(r) = r.filter(|r| (r.bytes.len() as u64) < item.floor_len()) {
                            item.floor = Some(r);
                        }
                    }
                    alloc = self.allocate(media_budget);
                }
            }

            let budgets = match alloc {
                Allocation::TooBig { .. } => {
                    return Ok(OfficeOutcome::Refused {
                        smallest_bytes: self.fixed_bytes + self.sum_floor(),
                    });
                }
                Allocation::AllLossless(b) | Allocation::Budgets(b) => b,
            };

            // Round 1: encode every item to its budget.
            for (k, (_, b)) in budgets.iter().enumerate() {
                let b = *b;
                self.items[k].budget = b;
                let item = &self.items[k];
                let chosen = if b >= item.lossless_len() {
                    item.lossless.clone()
                } else if b <= item.floor_len() {
                    item.smallest()
                } else {
                    let r = self.encode(k, Some(b), Mode::Fit)?;
                    r.filter(|r| (r.bytes.len() as u64) <= b)
                        .or_else(|| item.smallest())
                };
                self.items[k].chosen = chosen;
            }

            // Round 2: leftover of 5 percent or more goes to items below Good.
            let used: u64 = self.items.iter().map(Item::chosen_len).sum();
            let leftover = media_budget.saturating_sub(used);
            if leftover > 0 && leftover as f64 >= media_budget as f64 * ROUND2_MIN_LEFTOVER {
                let below_good: Vec<usize> = (0..self.items.len())
                    .filter(|&k| {
                        let it = &self.items[k];
                        it.chosen
                            .as_ref()
                            .is_some_and(|r| r.quality_label < QualityLabel::Good)
                            && it.chosen_len() < it.lossless_len()
                    })
                    .collect();
                let denom: u64 = below_good.iter().map(|&k| self.items[k].budget).sum();
                if denom > 0 {
                    for &k in &below_good {
                        let extra = (u128::from(leftover) * u128::from(self.items[k].budget)
                            / u128::from(denom)) as u64;
                        let new_budget = self.items[k].budget + extra;
                        let r = self.encode(k, Some(new_budget), Mode::Fit)?;
                        let item = &mut self.items[k];
                        if let Some(r) = r.filter(|r| {
                            let n = r.bytes.len() as u64;
                            n <= new_budget && n > item.chosen_len()
                        }) {
                            item.chosen = Some(r);
                            item.budget = new_budget;
                        }
                    }
                }
            }
        }

        let out = self.build(false)?;
        let out = if out.len() as u64 > budget {
            // A recompressor overshot its budget; fall back to the floor.
            let floor = self.build(true)?;
            if floor.len() as u64 > budget {
                return Ok(OfficeOutcome::Refused {
                    smallest_bytes: floor.len() as u64,
                });
            }
            floor
        } else {
            out
        };

        if original.len() as u64 <= budget && !saves_enough(original.len(), out.len()) {
            return Ok(OfficeOutcome::KeptOriginal {
                reason: format!(
                    "already fits at {} bytes; rebuilt package would be {} bytes",
                    original.len(),
                    out.len()
                ),
            });
        }
        self.finish(original, out, Some(budget))
    }

    fn smaller(mut self, original: &[u8], mode: Mode) -> Result<OfficeOutcome, OfficeError> {
        for k in 0..self.items.len() {
            let r = self.encode(k, None, mode)?;
            let item = &mut self.items[k];
            item.chosen = r.filter(|r| (r.bytes.len() as u64) < item.original);
        }
        let out = self.build(false)?;
        if !saves_enough(original.len(), out.len()) {
            return Ok(OfficeOutcome::KeptOriginal {
                reason: format!(
                    "rebuilt package is {} bytes, original {} bytes: under 5 percent saved",
                    out.len(),
                    original.len()
                ),
            });
        }
        self.finish(original, out, None)
    }

    /// Assemble the package from the prepared parts and the items' chosen
    /// (or, with `use_floor`, smallest) encodings.
    fn build(&self, use_floor: bool) -> Result<Vec<u8>, OfficeError> {
        self.check_cancel()?;
        let quick = write::quick_options();
        let mut parts = self.parts.clone();
        for it in &self.items {
            let replacement = if use_floor {
                it.smallest()
            } else {
                it.chosen.clone()
            };
            let bytes = match &replacement {
                Some(r) => r.bytes.as_slice(),
                None => self.pkg.entries[it.index].bytes.as_slice(),
            };
            parts[it.index] = Some(write::compress(bytes, &quick));
        }
        write::write_zip(self.pkg, &parts)
    }

    fn finish(
        self,
        original: &[u8],
        out: Vec<u8>,
        hard_bytes: Option<u64>,
    ) -> Result<OfficeOutcome, OfficeError> {
        let verification = verify(original, &out, hard_bytes);
        if !verification.ok {
            return Err(OfficeError::Verification(verification));
        }
        let media_recompressed = self.items.iter().filter(|it| it.chosen.is_some()).count();
        let media_left = self.pkg.media.len() - media_recompressed;
        let predicted_quality = self
            .items
            .iter()
            .filter_map(|it| it.chosen.as_ref().map(|r| r.quality_label))
            .min()
            .unwrap_or(QualityLabel::Great);
        let videos_left = self.videos_left
            + self
                .items
                .iter()
                .filter(|it| it.kind == MediaKind::Video && it.chosen.is_none())
                .count();
        let mut notes = Vec::new();
        if videos_left > 0 {
            notes.push(format!(
                "{videos_left} video{} left as is",
                if videos_left == 1 { "" } else { "s" }
            ));
        }
        Ok(OfficeOutcome::Done(OfficeResult {
            bytes: out,
            media_recompressed,
            media_left,
            predicted_quality,
            notes,
            verification,
        }))
    }
}

fn saves_enough(original: usize, output: usize) -> bool {
    (output as f64) <= (original as f64) * (1.0 - MIN_SAVING)
}
