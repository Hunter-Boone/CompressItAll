//! Quality search in log-size space, candidate choice, downscale fallback,
//! Smaller mode (DESIGN.md 3.4.4 to 3.4.7).

use crate::candidates::{candidates_for, Candidate, OutputImageFormat};
use crate::classify::{classify, Class, Classification};
use crate::decode::DecodedImage;
use crate::encode::{attach_icc, encode, EncodeInput};
use crate::{CancelFn, ImageError, ProgressFn};
use cia_core::{QualityLabel, SmallerLevel};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Fit { budget_bytes: u64 },
    Smaller { level: SmallerLevel },
}

#[derive(Debug, Clone)]
pub struct ImageOptions {
    pub mode: Mode,
    pub allowed_formats: Vec<OutputImageFormat>,
    pub allow_format_change: bool,
    pub modern_formats: bool,
    pub flatten_transparency: bool,
    pub max_long_edge: Option<u32>,
    pub keep_photo_details: bool,
    pub keep_location: bool,
    /// Max total encodes across candidates (design: 24).
    pub max_encodes: u32,
}

impl ImageOptions {
    pub fn fit(budget_bytes: u64, allowed: &[&str]) -> Self {
        Self {
            mode: Mode::Fit { budget_bytes },
            allowed_formats: allowed
                .iter()
                .filter_map(|s| OutputImageFormat::parse(s))
                .collect(),
            allow_format_change: true,
            modern_formats: false,
            flatten_transparency: false,
            max_long_edge: None,
            keep_photo_details: false,
            keep_location: false,
            max_encodes: 24,
        }
    }
    pub fn smaller(level: SmallerLevel, allowed: &[&str]) -> Self {
        Self {
            mode: Mode::Smaller { level },
            allowed_formats: allowed
                .iter()
                .filter_map(|s| OutputImageFormat::parse(s))
                .collect(),
            allow_format_change: false,
            modern_formats: false,
            flatten_transparency: false,
            max_long_edge: None,
            keep_photo_details: false,
            keep_location: false,
            max_encodes: 24,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImageAttempt {
    pub candidate: Candidate,
    pub quality: Option<u8>,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub score: Option<f32>,
    pub elapsed_ms: u64,
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
#[cfg(target_arch = "wasm32")]
fn now_ms() -> u64 {
    0
}

#[derive(Debug, Clone)]
pub struct ImageResult {
    pub bytes: Vec<u8>,
    pub format: OutputImageFormat,
    pub candidate: Candidate,
    pub width: u32,
    pub height: u32,
    pub quality: Option<u8>,
    pub label: QualityLabel,
    pub score: Option<f32>,
    pub class: Class,
    pub attempts: Vec<ImageAttempt>,
    pub downscaled: bool,
}

#[derive(Debug, Clone)]
pub enum ImageOutcome {
    Encoded(ImageResult),
    /// Original already fits (or nothing was 5 percent smaller).
    KeptOriginal {
        attempts: Vec<ImageAttempt>,
    },
    Refused {
        smallest_bytes: u64,
        attempts: Vec<ImageAttempt>,
    },
}

/// Per-(candidate, quality, w, h) cache so no encode is ever repeated.
struct Encoder<'a> {
    input: EncodeInput<'a>,
    cache: HashMap<(Candidate, u8, u32, u32), Vec<u8>>,
    attempts: Vec<ImageAttempt>,
    encodes: u32,
    max_encodes: u32,
    cancel: CancelFn<'a>,
}

impl<'a> Encoder<'a> {
    fn run(&mut self, c: Candidate, q: u8) -> Result<&[u8], ImageError> {
        let key = (c, q, self.input.img.width, self.input.img.height);
        if !self.cache.contains_key(&key) {
            if (self.cancel)() {
                return Err(ImageError::Cancelled);
            }
            if self.encodes >= self.max_encodes {
                return Err(ImageError::Encoder("encode budget exhausted".into()));
            }
            self.encodes += 1;
            let t0 = now_ms();
            let bytes = encode(&self.input, c, q)?;
            self.attempts.push(ImageAttempt {
                candidate: c,
                quality: c.quality_range().map(|_| q),
                width: key.2,
                height: key.3,
                bytes: bytes.len() as u64,
                score: None,
                elapsed_ms: now_ms().saturating_sub(t0),
            });
            self.cache.insert(key, bytes);
        }
        Ok(self.cache.get(&key).unwrap())
    }
}

/// Highest quality whose size is under `budget` (3.4.4). None when q_min does not fit.
fn search_quality(
    enc: &mut Encoder,
    c: Candidate,
    budget: u64,
) -> Result<Option<(u8, u64)>, ImageError> {
    let (q_min, q_max) = c.quality_range().expect("lossy candidate");
    let s_hi = enc.run(c, q_max)?.len() as u64;
    if s_hi <= budget {
        return Ok(Some((q_max, s_hi)));
    }
    let s_lo = enc.run(c, q_min)?.len() as u64;
    if s_lo > budget {
        return Ok(None);
    }
    let (mut lo, mut hi) = (q_min, q_max);
    let (mut s_lo, mut s_hi) = (s_lo as f64, s_hi as f64);
    for _ in 0..6 {
        if hi - lo <= 1 {
            break;
        }
        let t = ((budget as f64).ln() - s_lo.ln()) / (s_hi.ln() - s_lo.ln());
        let q = ((lo as f64 + t * (hi - lo) as f64).round() as u8).clamp(lo + 1, hi - 1);
        let s = enc.run(c, q)?.len() as u64;
        if s <= budget {
            lo = q;
            s_lo = s as f64;
        } else {
            hi = q;
            s_hi = s as f64;
        }
    }
    Ok(Some((lo, s_lo as u64)))
}

fn label_for(c: Candidate, q: Option<u8>, score: Option<f32>, downscaled: bool) -> QualityLabel {
    if c.is_exact() {
        return QualityLabel::Great;
    }
    if !c.is_lossy() {
        return QualityLabel::Good; // palette-quantised
    }
    if let Some(s) = score {
        return if s >= 85.0 {
            QualityLabel::Great
        } else if s >= 70.0 {
            QualityLabel::Good
        } else {
            QualityLabel::Okay
        };
    }
    match q {
        Some(q) if q >= 85 && !downscaled => QualityLabel::Great,
        Some(q) if q >= 65 => QualityLabel::Good,
        _ => QualityLabel::Okay,
    }
}

struct Found {
    candidate: Candidate,
    quality: Option<u8>,
    bytes: Vec<u8>,
}

/// Cheap estimate of a lossless/palette candidate's full-size bytes from a quarter-scale encode
/// (1/16 of the pixels, scaled up with a margin). Used only to skip hopeless candidates.
fn estimate_lossless(img: &DecodedImage, cls: Class, c: Candidate) -> Option<u64> {
    if img.pixels() < 1_000_000 {
        return None;
    }
    let (w, h) = (img.width / 4, img.height / 4);
    if w < 16 || h < 16 {
        return None;
    }
    let small = crate::resize::downscale(img, w, h, false);
    let input = EncodeInput {
        img: &small,
        class: cls,
    };
    let bytes = crate::encode::encode_estimate(&input, c).ok()?.len() as u64;
    // Quarter scale under-represents detail; lossless sizes scale slightly worse than linearly.
    Some(bytes * 16 * 9 / 10)
}

type CandidateRun = Result<(Option<Found>, Vec<ImageAttempt>, u32), ImageError>;

/// Try every candidate at this size against `budget`; returns the winner per 3.4.5.
/// Candidates run in parallel on native (up to 3 at a time), each with its own encode cache;
/// attempts are merged back afterwards.
fn try_size(
    enc: &mut Encoder,
    cands: &[Candidate],
    budget: u64,
    progress: ProgressFn,
    base: f32,
) -> Result<Option<Found>, ImageError> {
    let cls = enc.input.class;
    // Pre-screen lossless/palette candidates with a cheap quarter-scale estimate.
    let mut todo: Vec<Candidate> = Vec::new();
    for &c in cands {
        if !c.is_lossy() {
            let t = now_ms();
            let est = estimate_lossless(enc.input.img, cls, c);
            log::debug!("estimate {:?} = {:?} in {} ms", c, est, now_ms() - t);
            if let Some(est) = est {
                if est > budget * 3 {
                    continue;
                }
            }
        }
        todo.push(c);
    }
    progress(base, "Trying formats");
    let img = enc.input.img;
    let max_encodes = enc.max_encodes;
    let cancel = enc.cancel;
    let run_one = |c: Candidate| -> CandidateRun {
        let mut e = Encoder {
            input: EncodeInput { img, class: cls },
            cache: HashMap::new(),
            attempts: Vec::new(),
            encodes: 0,
            max_encodes,
            cancel,
        };
        let found = if c.is_lossy() {
            match search_quality(&mut e, c, budget)? {
                Some((q, _)) => Some(Found {
                    candidate: c,
                    quality: Some(q),
                    bytes: e.run(c, q)?.to_vec(),
                }),
                None => None,
            }
        } else {
            let bytes = e.run(c, 0)?.to_vec();
            if (bytes.len() as u64) <= budget {
                Some(Found {
                    candidate: c,
                    quality: None,
                    bytes,
                })
            } else {
                None
            }
        };
        Ok((found, e.attempts, e.encodes))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let results: Vec<CandidateRun> = {
        use rayon::prelude::*;
        let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build();
        match pool {
            Ok(pool) => pool.install(|| todo.par_iter().map(|&c| run_one(c)).collect()),
            Err(_) => todo.iter().map(|&c| run_one(c)).collect(),
        }
    };
    #[cfg(target_arch = "wasm32")]
    let results: Vec<CandidateRun> = todo.iter().map(|&c| run_one(c)).collect();
    let mut fits: Vec<Found> = Vec::new();
    for r in results {
        let (found, attempts, encodes) = r?;
        enc.attempts.extend(attempts);
        enc.encodes += encodes;
        if let Some(f) = found {
            // A lossless candidate that fits wins immediately (3.4.5 rule 1), in candidate order.
            fits.push(f);
        }
    }
    if let Some(pos) = fits.iter().position(|f| f.candidate.is_exact()) {
        // A pixel-exact candidate that fits wins immediately (3.4.5 rule 1), in candidate order.
        let first = fits
            .iter()
            .filter(|f| f.candidate.is_exact())
            .min_by_key(|f| cands.iter().position(|c| *c == f.candidate))
            .map(|f| f.candidate)
            .unwrap();
        let idx = fits
            .iter()
            .position(|f| f.candidate == first)
            .unwrap_or(pos);
        return Ok(Some(fits.swap_remove(idx)));
    }
    if fits.is_empty() {
        return Ok(None);
    }
    if fits.len() == 1 {
        return Ok(fits.pop());
    }
    // Score with SSIMULACRA2 and pick the best; ties within 1.0 go to the more compatible format.
    let src = enc.input.img;
    let mut scored: Vec<(f32, usize)> = Vec::new();
    for (i, f) in fits.iter().enumerate() {
        let s = decode_for_score(&f.bytes)
            .and_then(|d| crate::score::ssimulacra2(src, &d))
            .unwrap_or(0.0);
        scored.push((s, i));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let best = scored[0].0;
    let mut winner = scored
        .iter()
        .filter(|(s, _)| best - s <= 1.0)
        .map(|&(_, i)| i)
        .collect::<Vec<_>>();
    winner.sort_by_key(|&i| fits[i].candidate.format().compat_rank());
    let idx = winner[0];
    let s = scored.iter().find(|(_, i)| *i == idx).map(|x| x.0);
    if let Some(a) = enc
        .attempts
        .iter_mut()
        .rev()
        .find(|a| a.candidate == fits[idx].candidate && a.quality == fits[idx].quality)
    {
        a.score = s;
    }
    Ok(Some(fits.swap_remove(idx)))
}

fn decode_for_score(bytes: &[u8]) -> Option<DecodedImage> {
    crate::decode::decode(bytes).ok()
}

/// Smallest size reachable at full size: min over lossy candidates at q_min (F_i for allocation).
pub fn floor_size(
    img: &DecodedImage,
    opts: &ImageOptions,
    cancel: CancelFn,
) -> Result<u64, ImageError> {
    let (work, cls, cands) = prepare(img, opts);
    let mut enc = Encoder {
        input: EncodeInput {
            img: &work,
            class: cls.class,
        },
        cache: HashMap::new(),
        attempts: vec![],
        encodes: 0,
        max_encodes: opts.max_encodes,
        cancel,
    };
    let mut best = u64::MAX;
    for c in cands {
        let q = c.quality_range().map(|r| r.0).unwrap_or(0);
        let n = enc.run(c, q)?.len() as u64;
        best = best.min(n);
    }
    Ok(if best == u64::MAX {
        img.source_bytes
    } else {
        best
    })
}

/// Best lossless/optimised size (L_i for allocation): the smallest pixel-exact encoding, the
/// original bytes when the source format is allowed, or the top-quality lossy encode otherwise.
pub fn lossless_size(
    img: &DecodedImage,
    opts: &ImageOptions,
    cancel: CancelFn,
) -> Result<u64, ImageError> {
    let (work, cls, cands) = prepare(img, opts);
    let mut enc = Encoder {
        input: EncodeInput {
            img: &work,
            class: cls.class,
        },
        cache: HashMap::new(),
        attempts: vec![],
        encodes: 0,
        max_encodes: opts.max_encodes,
        cancel,
    };
    let mut best = u64::MAX;
    let source_allowed = OutputImageFormat::parse(img.source.token())
        .is_some_and(|f| opts.allowed_formats.contains(&f));
    if source_allowed && work.width == img.width && work.height == img.height {
        best = img.source_bytes;
    }
    for c in cands.iter().filter(|c| c.is_exact()) {
        if let Some(est) = estimate_lossless(&work, cls.class, *c) {
            if est > best.saturating_mul(2) {
                continue;
            }
        }
        best = best.min(enc.run(*c, 0)?.len() as u64);
    }
    if best == u64::MAX {
        if let Some(c) = cands.iter().find(|c| c.is_lossy()) {
            best = enc
                .run(*c, c.quality_range().map(|r| r.1).unwrap_or(0))?
                .len() as u64;
        } else if let Some(c) = cands.first() {
            best = enc.run(*c, 0)?.len() as u64;
        } else {
            best = img.source_bytes;
        }
    }
    Ok(best.min(img.source_bytes.max(1)))
}

fn prepare(
    img: &DecodedImage,
    opts: &ImageOptions,
) -> (DecodedImage, Classification, Vec<Candidate>) {
    let mut work = if opts.flatten_transparency && img.has_alpha {
        img.flattened()
    } else {
        img.clone()
    };
    if let Some(max) = opts.max_long_edge {
        if work.long_edge() > max {
            let (w, h) = crate::resize::dims_for_long_edge(work.width, work.height, max);
            let cls = classify(&work);
            work = crate::resize::downscale(&work, w, h, cls.class == Class::Photo);
        }
    }
    let cls = classify(&work);
    let source_only = if opts.allow_format_change {
        None
    } else {
        OutputImageFormat::parse(img.source.token()).or(Some(OutputImageFormat::Png))
    };
    let cands = candidates_for(
        cls.class,
        work.has_alpha,
        &opts.allowed_formats,
        opts.modern_formats || matches!(opts.mode, Mode::Fit { .. }) && false,
        source_only,
    );
    (work, cls, cands)
}

/// The image planner entry point.
pub fn compress(
    img: &DecodedImage,
    source_bytes: &[u8],
    opts: &ImageOptions,
    cancel: CancelFn,
    progress: ProgressFn,
) -> Result<ImageOutcome, ImageError> {
    let t_start = now_ms();
    let (work, cls, cands) = prepare(img, opts);
    log::debug!(
        "prepare {} ms: class {:?} candidates {:?}",
        now_ms() - t_start,
        cls.class,
        cands
    );
    let downscaled_by_option = work.width != img.width || work.height != img.height;
    let mut enc = Encoder {
        input: EncodeInput {
            img: &work,
            class: cls.class,
        },
        cache: HashMap::new(),
        attempts: vec![],
        encodes: 0,
        max_encodes: opts.max_encodes,
        cancel,
    };
    let finish = |enc: &Encoder, found: Found, w: u32, h: u32, downscaled: bool| -> ImageResult {
        let score = enc
            .attempts
            .iter()
            .rev()
            .find(|a| a.candidate == found.candidate && a.quality == found.quality)
            .and_then(|a| a.score);
        let label = label_for(found.candidate, found.quality, score, downscaled);
        let mut bytes = attach_icc(found.bytes, img.icc.as_deref(), found.candidate);
        if opts.keep_photo_details {
            bytes = crate::metadata::write_kept_exif(
                bytes,
                source_bytes,
                opts.keep_location,
                found.candidate.format(),
            );
        }
        ImageResult {
            bytes,
            format: found.candidate.format(),
            candidate: found.candidate,
            width: w,
            height: h,
            quality: found.quality,
            label,
            score,
            class: cls.class,
            attempts: enc.attempts.clone(),
            downscaled,
        }
    };

    match &opts.mode {
        Mode::Fit { budget_bytes } => {
            let budget = *budget_bytes;
            // Original already fits: only the cheap lossless step (PNG) runs; JPEG is left alone.
            if img.source_bytes <= budget
                && !downscaled_by_option
                && !(opts.flatten_transparency && img.has_alpha)
            {
                if img.source == crate::SourceFormat::Png && cands.contains(&Candidate::PngLossless)
                {
                    let n = enc.run(Candidate::PngLossless, 0)?.len() as u64;
                    if n * 100 <= img.source_bytes * 95 && n <= budget {
                        let bytes = enc.run(Candidate::PngLossless, 0)?.to_vec();
                        return Ok(ImageOutcome::Encoded(finish(
                            &enc,
                            Found {
                                candidate: Candidate::PngLossless,
                                quality: None,
                                bytes,
                            },
                            work.width,
                            work.height,
                            false,
                        )));
                    }
                }
                return Ok(ImageOutcome::KeptOriginal {
                    attempts: enc.attempts.clone(),
                });
            }
            if cands.is_empty() {
                return Ok(ImageOutcome::Refused {
                    smallest_bytes: img.source_bytes,
                    attempts: vec![],
                });
            }
            let t = now_ms();
            let first = try_size(&mut enc, &cands, budget, progress, 0.0)?;
            log::debug!("full-size pass {} ms", now_ms() - t);
            if let Some(found) = first {
                return Ok(ImageOutcome::Encoded(finish(
                    &enc,
                    found,
                    work.width,
                    work.height,
                    downscaled_by_option,
                )));
            }
            // Downscale, at most 3 rounds (3.4.6).
            let lossy: Vec<Candidate> = cands.iter().copied().filter(|c| c.is_lossy()).collect();
            let mut current = work.clone();
            let mut smallest = enc
                .attempts
                .iter()
                .map(|a| a.bytes)
                .min()
                .unwrap_or(img.source_bytes);
            for round in 0..3 {
                let best_floor = enc
                    .attempts
                    .iter()
                    .filter(|a| a.width == current.width && a.candidate.is_lossy())
                    .map(|a| a.bytes)
                    .min()
                    .unwrap_or(smallest);
                let scale = (budget as f64 / best_floor as f64).sqrt() * 0.97;
                let new_long = ((current.long_edge() as f64) * scale).floor() as u32;
                if new_long < 480 || lossy.is_empty() {
                    break;
                }
                let (w, h) =
                    crate::resize::dims_for_long_edge(current.width, current.height, new_long);
                progress(0.6 + round as f32 * 0.1, "Shrinking");
                current = crate::resize::downscale(&current, w, h, cls.class == Class::Photo);
                let mut sub = Encoder {
                    input: EncodeInput {
                        img: &current,
                        class: cls.class,
                    },
                    cache: HashMap::new(),
                    attempts: std::mem::take(&mut enc.attempts),
                    encodes: enc.encodes,
                    max_encodes: opts.max_encodes + 8,
                    cancel,
                };
                let res = try_size(&mut sub, &lossy, budget, progress, 0.6 + round as f32 * 0.1);
                enc.attempts = std::mem::take(&mut sub.attempts);
                enc.encodes = sub.encodes;
                smallest = smallest.min(
                    enc.attempts
                        .iter()
                        .map(|a| a.bytes)
                        .min()
                        .unwrap_or(smallest),
                );
                if let Some(found) = res? {
                    let r = finish(&enc, found, current.width, current.height, true);
                    return Ok(ImageOutcome::Encoded(r));
                }
            }
            Ok(ImageOutcome::Refused {
                smallest_bytes: smallest,
                attempts: enc.attempts.clone(),
            })
        }
        Mode::Smaller { level } => {
            let (target, q_lo, q_hi) = match level {
                SmallerLevel::KeepQuality => (85.0f32, 60u8, 92u8),
                SmallerLevel::Smallest => (70.0, 45, 85),
            };
            let mut best: Option<(Found, Option<f32>)> = None;
            // Lossless candidates always run for Graphic (and PNG sources).
            for &c in cands.iter().filter(|c| !c.is_lossy()) {
                let bytes = enc.run(c, 0)?.to_vec();
                if best
                    .as_ref()
                    .is_none_or(|(b, _)| bytes.len() < b.bytes.len())
                {
                    best = Some((
                        Found {
                            candidate: c,
                            quality: None,
                            bytes,
                        },
                        None,
                    ));
                }
            }
            for &c in cands.iter().filter(|c| c.is_lossy()) {
                // Search the lowest quality whose score is at least `target`, log-interpolated on score.
                let (mut lo, mut hi) = (q_lo, q_hi);
                let found: Option<(u8, f32)>;
                let score_at = |enc: &mut Encoder, q: u8| -> Result<f32, ImageError> {
                    let bytes = enc.run(c, q)?.to_vec();
                    let s = decode_for_score(&bytes)
                        .and_then(|d| crate::score::ssimulacra2(&work, &d))
                        .unwrap_or(0.0);
                    if let Some(a) = enc
                        .attempts
                        .iter_mut()
                        .rev()
                        .find(|a| a.candidate == c && a.quality == Some(q))
                    {
                        a.score = Some(s);
                    }
                    Ok(s)
                };
                let s_hi = score_at(&mut enc, hi)?;
                if s_hi < target {
                    found = Some((hi, s_hi));
                } else {
                    let s_lo = score_at(&mut enc, lo)?;
                    if s_lo >= target {
                        found = Some((lo, s_lo));
                    } else {
                        let (mut s_lo, mut s_hi) = (s_lo, s_hi);
                        for _ in 0..5 {
                            if hi - lo <= 1 {
                                break;
                            }
                            let t = ((target - s_lo) / (s_hi - s_lo).max(0.01)).clamp(0.1, 0.9);
                            let q = ((lo as f32 + t * (hi - lo) as f32).round() as u8)
                                .clamp(lo + 1, hi - 1);
                            let s = score_at(&mut enc, q)?;
                            if s >= target {
                                hi = q;
                                s_hi = s;
                            } else {
                                lo = q;
                                s_lo = s;
                            }
                        }
                        found = Some((hi, s_hi));
                    }
                }
                if let Some((q, s)) = found {
                    let bytes = enc.run(c, q)?.to_vec();
                    if best
                        .as_ref()
                        .is_none_or(|(b, _)| bytes.len() < b.bytes.len())
                    {
                        best = Some((
                            Found {
                                candidate: c,
                                quality: Some(q),
                                bytes,
                            },
                            Some(s),
                        ));
                    }
                }
            }
            match best {
                Some((f, _)) if (f.bytes.len() as u64) * 100 <= img.source_bytes * 95 => {
                    let r = finish(&enc, f, work.width, work.height, downscaled_by_option);
                    Ok(ImageOutcome::Encoded(r))
                }
                _ => Ok(ImageOutcome::KeptOriginal {
                    attempts: enc.attempts.clone(),
                }),
            }
        }
    }
}
