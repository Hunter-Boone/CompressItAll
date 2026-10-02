//! Preview planning (DESIGN.md 3.1, 3.9.1, 3.9.2): per-item predictions,
//! per-message allocation, packaging decision and the headline.

use crate::planners::{self, Ctx, Sizes};
use crate::{Engine, EngineError};
use cia_core::allocation::{allocate, Allocation, ItemSizes};
use cia_core::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub items: Vec<InputItem>,
    pub goal: Goal,
    pub packaging: Packaging,
    pub options: JobOptions,
}

/// What the preset allows for this job (from the preset id in the goal).
pub struct Allowed {
    pub image: Vec<String>,
    pub audio: Vec<String>,
    pub video: Vec<cia_core::presets::VideoFormat>,
    pub animated: Vec<String>,
    pub video_caps: cia_core::presets::VideoCaps,
}

pub fn allowed_for(goal: &Goal) -> Allowed {
    let preset_id = match goal {
        Goal::Fit { preset_id, .. } => preset_id.as_str(),
        Goal::Smaller { .. } => "smaller",
    };
    let p = cia_core::presets::find(preset_id)
        .or_else(|_| cia_core::presets::find("custom"))
        .expect("presets embedded");
    Allowed {
        image: p.formats.image.clone(),
        audio: p.formats.audio.clone(),
        video: p.formats.video.clone(),
        animated: p.formats.animated.clone(),
        video_caps: p.video_caps.clone(),
    }
}

/// Per-item sizes for allocation; images get real encodes, the rest are estimates.
pub(crate) fn item_sizes(engine: &Engine, item: &InputItem, ctx: &Ctx) -> Sizes {
    let original = item.bytes;
    match item.kind {
        Kind::Image => match engine
            .reader
            .read(&item.source)
            .ok()
            .and_then(|b| planners::image::sizes(&b, ctx).ok())
        {
            Some((s, _)) => s,
            None => Sizes {
                lossless: original,
                floor: original,
            },
        },
        Kind::AnimatedImage => Sizes {
            lossless: original,
            floor: original / 6,
        },
        Kind::Video => {
            let dur_s = item.detail.duration_ms.unwrap_or(10_000) as f64 / 1000.0;
            let floor = ((640.0 * 360.0 * 30.0 * 0.07 + 32_000.0) * dur_s / 8.0) as u64;
            Sizes {
                lossless: original,
                floor: floor.min(original),
            }
        }
        Kind::Audio => {
            // "Lossless" for allocation means the best quality we would want: the top rung of an
            // allowed lossy format when the source must be converted, otherwise the original bytes.
            let dur_s = item.detail.duration_ms.unwrap_or(10_000) as f64 / 1000.0;
            let codec = item
                .detail
                .audio_streams
                .first()
                .map(|a| a.codec.as_str())
                .unwrap_or("");
            let allowed_as_is = ctx
                .allowed_audio
                .iter()
                .any(|f| f == native_audio_id(codec));
            let top = ((192_000.0 * dur_s) / 8.0) as u64 + 4096;
            Sizes {
                lossless: if allowed_as_is { original } else { top },
                floor: ((32_000.0 * dur_s) / 8.0) as u64,
            }
        }
        Kind::Pdf | Kind::OfficeDoc => Sizes {
            lossless: original * 9 / 10,
            floor: original / 8,
        },
        Kind::Archive => Sizes {
            lossless: original,
            floor: original * 7 / 10,
        },
        Kind::Text => Sizes {
            lossless: original / 4,
            floor: original / 4,
        },
        Kind::Other => Sizes {
            lossless: original,
            floor: original,
        },
    }
}

/// Budget per item: per-file limits give each item the whole budget; per-message limits water-fill.
pub(crate) fn budgets(
    engine: &Engine,
    req: &PlanRequest,
    ctx: &Ctx,
) -> Result<(Vec<Option<u64>>, Option<Refusal>), EngineError> {
    let Some(limit) = req.goal.limit() else {
        return Ok((vec![None; req.items.len()], None));
    };
    if limit.scope == LimitScope::PerFile {
        return Ok((
            req.items
                .iter()
                .map(|i| Some(limit.budget_for(i.kind)))
                .collect(),
            None,
        ));
    }
    let names: Vec<&str> = req.items.iter().map(|i| i.rel_path.as_str()).collect();
    let packaging_overhead = if will_zip(req, limit) {
        cia_core::zip_overhead::zip_overhead_bytes(
            names.iter().copied(),
            req.items.iter().map(|i| i.bytes).max().unwrap_or(0),
        )
    } else {
        0
    };
    let total = limit.raw_budget_bytes.saturating_sub(packaging_overhead);
    let sizes: Vec<ItemSizes> = req
        .items
        .iter()
        .map(|i| {
            let s = item_sizes(engine, i, ctx);
            ItemSizes {
                id: i.id.clone(),
                lossless: s.lossless,
                floor: s.floor,
            }
        })
        .collect();
    match allocate(&sizes, total) {
        Allocation::AllLossless(v) | Allocation::Budgets(v) => {
            // Each item still has to be under its own per-kind hard limit.
            Ok((
                v.into_iter()
                    .zip(&req.items)
                    .map(|((_, b), i)| Some(b.min(limit.budget_for(i.kind))))
                    .collect(),
                None,
            ))
        }
        Allocation::TooBig { split, remove } => {
            let smallest: u64 = sizes.iter().map(|s| s.floor).sum();
            let message = cia_core::copy::refusal_message(
                &RefusalCode::TotalTooBig,
                Some(limit.hard_bytes),
                Some(smallest),
                Kind::Other,
            );
            let mut suggestions = vec![];
            if split.len() > 1 {
                suggestions.push(Suggestion::SplitIntoMessages { groups: split });
            }
            if !remove.is_empty() && remove.len() < req.items.len() {
                suggestions.push(Suggestion::RemoveFiles { item_ids: remove });
            }
            Ok((
                vec![None; req.items.len()],
                Some(Refusal {
                    code: RefusalCode::TotalTooBig,
                    message,
                    smallest_bytes: Some(smallest),
                    suggestions,
                }),
            ))
        }
    }
}

pub(crate) fn will_zip(req: &PlanRequest, limit: &ResolvedLimit) -> bool {
    match req.packaging {
        Packaging::SeparateFiles => false,
        Packaging::Zip | Packaging::SevenZip | Packaging::TarZst | Packaging::TarXz => true,
        Packaging::Auto => {
            let n = req.items.len();
            if n <= 1 {
                return false;
            }
            let max = limit.max_files_per_message.unwrap_or(10) as usize;
            n > max
        }
    }
}

pub(crate) fn archive_name(req: &PlanRequest, label: &str, packaging: Packaging) -> String {
    let folder = req.items.iter().find_map(|i| i.folder.clone());
    let stem = folder.unwrap_or_else(|| {
        let first = req
            .items
            .first()
            .map(|i| cia_core::naming::split_stem(i.file_name()).0)
            .unwrap_or_else(|| "Files".into());
        if req.items.len() > 1 {
            format!("{first} and {} more", req.items.len() - 1)
        } else {
            first
        }
    });
    let ext = match packaging {
        Packaging::SevenZip => "7z",
        Packaging::TarZst => "tar.zst",
        Packaging::TarXz => "tar.xz",
        _ => "zip",
    };
    cia_core::naming::output_name(&stem, label, Some(ext), 1)
}

impl Engine {
    /// The prediction shown before Compress.
    pub fn preview(
        &self,
        req: &PlanRequest,
        cancel: &crate::CancelToken,
    ) -> Result<Plan, EngineError> {
        let allowed = allowed_for(&req.goal);
        let limit = req.goal.limit().cloned();
        let smaller = match &req.goal {
            Goal::Smaller { level } => Some(*level),
            _ => None,
        };
        let cancel_fn = || cancel.is_cancelled();
        let progress = |_: f32, _: &str| {};
        let ctx = Ctx {
            options: &req.options,
            allowed_image: allowed.image.clone(),
            allowed_audio: allowed.audio.clone(),
            hard_bytes: limit.as_ref().map(|l| l.hard_bytes),
            smaller,
            cancel: &cancel_fn,
            progress: &progress,
        };
        let (budgets, total_refusal) = budgets(self, req, &ctx)?;
        let label = limit
            .as_ref()
            .map(|l| l.output_label.clone())
            .unwrap_or_else(|| "smaller".into());
        let mut items = Vec::new();
        let mut worst: Option<QualityLabel> = None;
        let mut uncertain = false;
        let mut refusal: Option<Refusal> = total_refusal;
        let audio_caps = planners::audio::host_caps(&self.caps);
        for (item, budget) in req.items.iter().zip(budgets.iter()) {
            let hard = limit.as_ref().map(|l| l.hard_for(item.kind));
            let ictx = Ctx {
                hard_bytes: hard,
                ..ctx_clone(&ctx)
            };
            let (prediction, strategy, item_refusal) =
                self.predict_item(item, *budget, &ictx, &allowed, &audio_caps);
            if let Some(q) = prediction.quality {
                worst = Some(worst.map_or(q, |w| w.min(q)));
            }
            if item.kind == Kind::Video {
                uncertain = true;
            }
            if refusal.is_none() {
                refusal = item_refusal;
            }
            items.push(ItemPlan {
                item_id: item.id.clone(),
                budget_bytes: *budget,
                strategy,
                prediction,
            });
        }
        let packaging = match (&limit, req.packaging) {
            (Some(l), p) if will_zip(req, l) => {
                let fmt = if p == Packaging::Auto {
                    Packaging::Zip
                } else {
                    p
                };
                let names: Vec<&str> = req.items.iter().map(|i| i.rel_path.as_str()).collect();
                PackagingPlan::Archive {
                    format: fmt,
                    file_name: archive_name(req, &label, fmt),
                    overhead_bytes: cia_core::zip_overhead::zip_overhead_bytes(names, 0),
                }
            }
            (None, p)
                if matches!(
                    p,
                    Packaging::Zip | Packaging::SevenZip | Packaging::TarZst | Packaging::TarXz
                ) && !req.items.is_empty() =>
            {
                PackagingPlan::Archive {
                    format: p,
                    file_name: archive_name(req, &label, p),
                    overhead_bytes: 0,
                }
            }
            _ => PackagingPlan::SeparateFiles,
        };
        let predicted_total: u64 = items
            .iter()
            .map(|p| p.prediction.predicted_bytes)
            .sum::<u64>()
            + match &packaging {
                PackagingPlan::Archive { overhead_bytes, .. } => *overhead_bytes,
                _ => 0,
            };
        let verdict = match refusal {
            Some(r) => PlanVerdict::CannotFit { refusal: r },
            None if uncertain => PlanVerdict::Uncertain {
                quality: worst.unwrap_or(QualityLabel::Great),
            },
            None => PlanVerdict::WillFit {
                quality: worst.unwrap_or(QualityLabel::Great),
            },
        };
        let headline = cia_core::copy::plan_headline(&items, &req.items, &req.goal, &verdict);
        Ok(Plan {
            job_id: cia_core::new_id(),
            items,
            packaging,
            predicted_total_bytes: predicted_total,
            verdict,
            headline,
        })
    }

    fn predict_item(
        &self,
        item: &InputItem,
        budget: Option<u64>,
        ctx: &Ctx,
        allowed: &Allowed,
        audio_caps: &cia_audio::plan::HostAudioCaps,
    ) -> (Prediction, Strategy, Option<Refusal>) {
        let hard = ctx.hard_bytes;
        let refuse = |code: RefusalCode, smallest: Option<u64>, suggestions: Vec<Suggestion>| {
            let message = cia_core::copy::refusal_message(&code, hard, smallest, item.kind);
            Refusal {
                code,
                message,
                smallest_bytes: smallest,
                suggestions,
            }
        };
        if item.detail.format == "corrupt" || item.detail.format == "unreadable" {
            return (
                Prediction {
                    predicted_bytes: item.bytes,
                    exact: true,
                    summary: "Damaged".into(),
                    quality: None,
                    notes: vec![cia_core::copy::failure_message("damaged_input", None, None)],
                },
                Strategy::None,
                None,
            );
        }
        if self.needs_ffmpeg(item) {
            return (
                Prediction {
                    predicted_bytes: item.bytes,
                    exact: false,
                    summary: "Needs video support".into(),
                    quality: None,
                    notes: vec![],
                },
                Strategy::None,
                Some(refuse(
                    RefusalCode::NeedsFfmpeg,
                    None,
                    vec![Suggestion::InstallFfmpeg],
                )),
            );
        }
        if item.detail.encrypted == Some(true) && matches!(item.kind, Kind::Pdf | Kind::OfficeDoc) {
            return (
                Prediction {
                    predicted_bytes: item.bytes,
                    exact: true,
                    summary: "Password-protected".into(),
                    quality: None,
                    notes: vec![],
                },
                Strategy::None,
                Some(refuse(RefusalCode::Encrypted, None, vec![])),
            );
        }
        // Already fits and nothing to do.
        let fits_already =
            budget.is_some_and(|b| item.bytes < b) && hard.is_some_and(|h| item.bytes < h);
        match item.kind {
            Kind::Image => {
                if fits_already && item.detail.format == "jpeg" {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: dims(item, "JPEG"),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                let bytes = match self.reader.read(&item.source) {
                    Ok(b) => b,
                    Err(_) => {
                        return (
                            Prediction {
                                predicted_bytes: item.bytes,
                                exact: false,
                                summary: "Unreadable".into(),
                                quality: None,
                                notes: vec![],
                            },
                            Strategy::None,
                            None,
                        )
                    }
                };
                // Real pass: the prediction for images is the real result size (3.4: "images run their real pass-0 encodes").
                // The result is cached so `run` does not encode it a second time.
                let key = cache_key(item, budget, ctx);
                let cached = self.preview_cache.lock().unwrap().get(&key).cloned();
                let outcome = match cached {
                    Some(e) => planners::PlannerOutcome::Encoded(e),
                    None => planners::image::run(item, &bytes, budget, ctx),
                };
                match outcome {
                    planners::PlannerOutcome::Encoded(e) => {
                        let pred = Prediction {
                            predicted_bytes: e.bytes.len() as u64,
                            exact: true,
                            summary: e.summary.clone(),
                            quality: e.quality,
                            notes: vec![],
                        };
                        let mut c = self.preview_cache.lock().unwrap();
                        if c.len() > 64 {
                            c.clear();
                        }
                        c.insert(key, e);
                        (
                            pred,
                            Strategy::Image {
                                candidates: ctx.allowed_image.clone(),
                                max_long_edge: ctx.options.max_long_edge,
                            },
                            None,
                        )
                    }
                    planners::PlannerOutcome::KeptOriginal { .. } => (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: dims(item, &item.detail.format.to_uppercase()),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    ),
                    planners::PlannerOutcome::Refused {
                        code,
                        smallest_bytes,
                        ..
                    } => (
                        Prediction {
                            predicted_bytes: smallest_bytes.unwrap_or(item.bytes),
                            exact: true,
                            summary: String::new(),
                            quality: None,
                            notes: vec![],
                        },
                        Strategy::None,
                        Some(refuse(
                            code,
                            smallest_bytes,
                            self.preset_suggestions(item, smallest_bytes),
                        )),
                    ),
                    planners::PlannerOutcome::Failed { code, .. } => (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: false,
                            summary: String::new(),
                            quality: None,
                            notes: vec![cia_core::copy::failure_message(code, None, None)],
                        },
                        Strategy::None,
                        None,
                    ),
                }
            }
            Kind::AnimatedImage => {
                if fits_already {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "GIF".into(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                (
                    Prediction {
                        predicted_bytes: budget.map(|b| b * 9 / 10).unwrap_or(item.bytes * 2 / 3),
                        exact: false,
                        summary: "GIF".into(),
                        quality: Some(QualityLabel::Good),
                        notes: vec![],
                    },
                    Strategy::AnimatedImage {
                        candidates: vec!["gif".into()],
                    },
                    None,
                )
            }
            Kind::Video => self.predict_video(item, budget, ctx, allowed, refuse),
            Kind::Audio => {
                let info = self
                    .reader
                    .read(&item.source)
                    .ok()
                    .and_then(|b| cia_audio::probe(&b).ok());
                let Some(info) = info else {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: false,
                            summary: "Audio".into(),
                            quality: None,
                            notes: vec![],
                        },
                        Strategy::None,
                        None,
                    );
                };
                if fits_already
                    && (ctx
                        .allowed_audio
                        .iter()
                        .any(|f| f == native_audio_id(&info.codec))
                        || budget.is_none())
                {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: info.codec.to_uppercase(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                match planners::audio::predict(&info, item.bytes, budget, audio_caps, ctx) {
                    Ok((bytes, summary, q)) => (
                        Prediction {
                            predicted_bytes: bytes,
                            exact: false,
                            summary,
                            quality: Some(q),
                            notes: vec![],
                        },
                        Strategy::Audio {
                            format: "auto".into(),
                            bitrate_bps: 0,
                            channels: info.channels as u32,
                        },
                        None,
                    ),
                    Err(code) => {
                        let sugg = match &code {
                            RefusalCode::TooLongForLimit { max_duration_ms } => {
                                vec![Suggestion::Trim {
                                    max_duration_ms: *max_duration_ms,
                                }]
                            }
                            _ => vec![],
                        };
                        (
                            Prediction {
                                predicted_bytes: item.bytes,
                                exact: false,
                                summary: String::new(),
                                quality: None,
                                notes: vec![],
                            },
                            Strategy::None,
                            Some(refuse(code, None, sugg)),
                        )
                    }
                }
            }
            Kind::Pdf => {
                if fits_already {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: pages(item),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                let predicted = budget
                    .map(|b| b.min(item.bytes) * 85 / 100)
                    .unwrap_or(item.bytes * 7 / 10);
                (
                    Prediction {
                        predicted_bytes: predicted,
                        exact: false,
                        summary: pages(item),
                        quality: Some(if budget.is_some_and(|b| b < item.bytes / 4) {
                            QualityLabel::Okay
                        } else {
                            QualityLabel::Good
                        }),
                        notes: vec![],
                    },
                    Strategy::Pdf {
                        lossless_only: false,
                    },
                    None,
                )
            }
            Kind::OfficeDoc => {
                if fits_already {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "Document".into(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                let predicted = budget
                    .map(|b| b.min(item.bytes) * 85 / 100)
                    .unwrap_or(item.bytes * 6 / 10);
                (
                    Prediction {
                        predicted_bytes: predicted,
                        exact: false,
                        summary: "Photos re-saved at good quality".into(),
                        quality: Some(QualityLabel::Good),
                        notes: vec![],
                    },
                    Strategy::Office,
                    None,
                )
            }
            Kind::Archive => {
                if item.detail.format == "rar" {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "RAR".into(),
                            quality: None,
                            notes: vec![],
                        },
                        Strategy::None,
                        Some(refuse(
                            RefusalCode::UnsupportedInput { what: "rar".into() },
                            None,
                            vec![],
                        )),
                    );
                }
                if item.detail.encrypted == Some(true) {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "Zip".into(),
                            quality: None,
                            notes: vec![],
                        },
                        Strategy::None,
                        Some(refuse(RefusalCode::Encrypted, None, vec![])),
                    );
                }
                if fits_already {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "Zip".into(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                (
                    Prediction {
                        predicted_bytes: budget.map(|b| b * 9 / 10).unwrap_or(item.bytes * 7 / 10),
                        exact: false,
                        summary: "Zip".into(),
                        quality: Some(QualityLabel::Good),
                        notes: vec![],
                    },
                    Strategy::Archive {
                        optimise_inside: ctx.options.optimise_inside_archives,
                    },
                    None,
                )
            }
            Kind::Text => {
                if fits_already || budget.is_none() {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "Text".into(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                let z = item.bytes / 4;
                if budget.is_some_and(|b| z >= b) {
                    return (
                        Prediction {
                            predicted_bytes: z,
                            exact: false,
                            summary: "Text".into(),
                            quality: None,
                            notes: vec![],
                        },
                        Strategy::None,
                        Some(refuse(
                            RefusalCode::CannotShrinkType,
                            Some(z),
                            self.preset_suggestions(item, Some(z)),
                        )),
                    );
                }
                (
                    Prediction {
                        predicted_bytes: z,
                        exact: false,
                        summary: "zipped".into(),
                        quality: Some(QualityLabel::Great),
                        notes: vec![],
                    },
                    Strategy::ZipWrap,
                    None,
                )
            }
            Kind::Other => {
                if fits_already || budget.is_none() {
                    return (
                        Prediction {
                            predicted_bytes: item.bytes,
                            exact: true,
                            summary: "File".into(),
                            quality: Some(QualityLabel::Great),
                            notes: vec![],
                        },
                        Strategy::Copy,
                        None,
                    );
                }
                (
                    Prediction {
                        predicted_bytes: item.bytes,
                        exact: true,
                        summary: "File".into(),
                        quality: None,
                        notes: vec![],
                    },
                    Strategy::None,
                    Some(refuse(
                        RefusalCode::CannotShrinkType,
                        Some(item.bytes),
                        self.preset_suggestions(item, Some(item.bytes)),
                    )),
                )
            }
        }
    }

    fn predict_video(
        &self,
        item: &InputItem,
        budget: Option<u64>,
        ctx: &Ctx,
        allowed: &Allowed,
        refuse: impl Fn(RefusalCode, Option<u64>, Vec<Suggestion>) -> Refusal,
    ) -> (Prediction, Strategy, Option<Refusal>) {
        let Some(video) = &self.video else {
            return (
                Prediction {
                    predicted_bytes: item.bytes,
                    exact: false,
                    summary: "Needs video support".into(),
                    quality: None,
                    notes: vec![],
                },
                Strategy::None,
                Some(refuse(
                    RefusalCode::NeedsFfmpeg,
                    None,
                    vec![Suggestion::InstallFfmpeg],
                )),
            );
        };
        let probe = match video.probe(&item.source) {
            Ok(p) => p,
            Err(_) => {
                return (
                    Prediction {
                        predicted_bytes: item.bytes,
                        exact: false,
                        summary: "Damaged".into(),
                        quality: None,
                        notes: vec![cia_core::copy::failure_message("damaged_video", None, None)],
                    },
                    Strategy::None,
                    None,
                )
            }
        };
        let encoder = video.encoder_kind(ctx.options.video.faster);
        let target = video_target(&probe, ctx, allowed);
        let popts = plan_options(item, ctx);
        let Some(b) = budget else {
            let plan = cia_video_plan::plan(
                &probe,
                &cia_video_plan::Budget {
                    raw_budget_bytes: item.bytes * 6 / 10,
                    hard_bytes: item.bytes,
                    safety_bytes: 0,
                },
                &target,
                encoder,
                &popts,
            );
            return match plan {
                Ok(p) => (
                    Prediction {
                        predicted_bytes: p.predicted_bytes,
                        exact: false,
                        summary: video_summary(&p),
                        quality: Some(p.quality),
                        notes: vec![],
                    },
                    video_strategy(&p, encoder),
                    None,
                ),
                Err(_) => (
                    Prediction {
                        predicted_bytes: item.bytes,
                        exact: true,
                        summary: "Already small".into(),
                        quality: Some(QualityLabel::Great),
                        notes: vec![],
                    },
                    Strategy::Copy,
                    None,
                ),
            };
        };
        let limit = ctx.hard_bytes.unwrap_or(b);
        let budget_s = cia_video_plan::Budget {
            raw_budget_bytes: b,
            hard_bytes: limit,
            safety_bytes: limit.saturating_sub(b).min(limit),
        };
        if let cia_video_plan::Decision::KeepOriginal =
            cia_video_plan::keep_original_or_remux(&probe, limit, item.bytes, &allowed.video)
        {
            return (
                Prediction {
                    predicted_bytes: item.bytes,
                    exact: true,
                    summary: video_probe_summary(&probe),
                    quality: Some(QualityLabel::Great),
                    notes: vec![],
                },
                Strategy::Copy,
                None,
            );
        }
        match cia_video_plan::plan(&probe, &budget_s, &target, encoder, &popts) {
            Ok(p) => (
                Prediction {
                    predicted_bytes: p.predicted_bytes,
                    exact: false,
                    summary: video_summary(&p),
                    quality: Some(p.quality),
                    notes: if probe.is_hdr {
                        vec!["HDR video will be converted to standard colour.".into()]
                    } else {
                        vec![]
                    },
                },
                video_strategy(&p, encoder),
                None,
            ),
            Err(r) => {
                let mut sugg = match &r.code {
                    RefusalCode::TooLongForLimit { max_duration_ms } => vec![Suggestion::Trim {
                        max_duration_ms: *max_duration_ms,
                    }],
                    _ => vec![],
                };
                for p in cia_core::presets::all()
                    .iter()
                    .filter(|p| p.mode == cia_core::presets::PresetMode::Fit)
                {
                    if let Some(pp) = cia_video_plan::predict_for_preset(&probe, p, encoder) {
                        if pp.quality >= QualityLabel::Okay
                            && Some(p.id.as_str())
                                != match &ctx_goal_preset(ctx) {
                                    Some(s) => Some(s.as_str()),
                                    None => None,
                                }
                        {
                            sugg.push(Suggestion::PickPreset {
                                preset_id: p.id.clone(),
                                predicted_bytes: pp.predicted_bytes,
                            });
                        }
                    }
                    if sugg.len() >= 3 {
                        break;
                    }
                }
                (
                    Prediction {
                        predicted_bytes: item.bytes,
                        exact: false,
                        summary: String::new(),
                        quality: None,
                        notes: vec![],
                    },
                    Strategy::None,
                    Some(refuse(r.code, None, sugg)),
                )
            }
        }
    }

    fn preset_suggestions(&self, item: &InputItem, smallest: Option<u64>) -> Vec<Suggestion> {
        let Some(s) = smallest else { return vec![] };
        let mut out = Vec::new();
        for p in cia_core::presets::all()
            .iter()
            .filter(|p| p.mode == cia_core::presets::PresetMode::Fit)
        {
            if let Ok(l) = p.resolve() {
                if l.budget_for(item.kind) > s * 11 / 10 {
                    out.push(Suggestion::PickPreset {
                        preset_id: p.id.clone(),
                        predicted_bytes: s,
                    });
                }
            }
            if out.len() >= 2 {
                break;
            }
        }
        out
    }
}

fn ctx_goal_preset(_ctx: &Ctx) -> Option<String> {
    None
}

/// Cache key for an image encode: item, budget, and every option that changes the result.
pub(crate) fn cache_key(item: &InputItem, budget: Option<u64>, ctx: &Ctx) -> String {
    let o = ctx.options;
    format!(
        "{}|{:?}|{:?}|{}|{}|{}|{}|{:?}|{}|{}|{:?}",
        item.id,
        budget,
        ctx.smaller,
        o.allow_format_change,
        o.modern_formats,
        o.flatten_transparency,
        o.keep_photo_details,
        o.max_long_edge,
        o.keep_location,
        ctx.allowed_image.join(","),
        ctx.hard_bytes
    )
}

pub(crate) fn ctx_clone<'a>(c: &Ctx<'a>) -> Ctx<'a> {
    Ctx {
        options: c.options,
        allowed_image: c.allowed_image.clone(),
        allowed_audio: c.allowed_audio.clone(),
        hard_bytes: c.hard_bytes,
        smaller: c.smaller,
        cancel: c.cancel,
        progress: c.progress,
    }
}

fn dims(item: &InputItem, fmt: &str) -> String {
    match (item.detail.width, item.detail.height) {
        (Some(w), Some(h)) => format!("{w} × {h} {fmt}"),
        _ => fmt.to_string(),
    }
}
fn pages(item: &InputItem) -> String {
    match item.detail.page_count {
        Some(1) => "1 page".into(),
        Some(n) => format!("{n} pages"),
        None => "PDF".into(),
    }
}
pub(crate) fn native_audio_id(codec: &str) -> &'static str {
    match codec {
        "mp3" => "mp3",
        "flac" => "flac",
        "opus" => "ogg_opus",
        "aac" => "m4a_aac",
        "pcm" => "wav",
        _ => "other",
    }
}

pub(crate) fn video_target(
    probe: &cia_video_plan::VideoProbe,
    ctx: &Ctx,
    allowed: &Allowed,
) -> cia_video_plan::Target {
    let want_webm = ctx.options.video.format == VideoFormatPref::SmallerFiles;
    let pick = allowed
        .video
        .iter()
        .find(|f| {
            if want_webm {
                f.container == "webm"
            } else {
                f.container == "mp4"
            }
        })
        .or_else(|| allowed.video.first())
        .cloned()
        .unwrap_or(cia_core::presets::VideoFormat {
            container: "mp4".into(),
            video: "h264".into(),
            audio: "aac".into(),
        });
    let _ = probe;
    cia_video_plan::Target::from_format(&pick, &allowed.video_caps).unwrap_or_else(|| {
        cia_video_plan::Target::from_format(
            &cia_core::presets::VideoFormat {
                container: "mp4".into(),
                video: "h264".into(),
                audio: "aac".into(),
            },
            &allowed.video_caps,
        )
        .expect("h264 mp4 parses")
    })
}

pub(crate) fn plan_options(item: &InputItem, ctx: &Ctx) -> cia_video_plan::PlanOptions {
    cia_video_plan::PlanOptions {
        audio: ctx.options.video.audio.clone(),
        frame_rate_pref: ctx.options.video.frame_rate.clone(),
        trim: ctx.options.video.trims.get(&item.id).copied(),
    }
}

pub(crate) fn video_summary(p: &cia_video_plan::VideoPlan) -> String {
    let codec = match p.video_codec {
        cia_video_plan::VideoCodec::H264 => "H.264",
        cia_video_plan::VideoCodec::Vp9 => "VP9",
        cia_video_plan::VideoCodec::Av1 => "AV1",
    };
    format!(
        "{}p, {} fps, {codec}",
        p.width.min(p.height),
        p.fps.round() as u32
    )
}
fn video_probe_summary(p: &cia_video_plan::VideoProbe) -> String {
    format!(
        "{}p, {} fps, {}",
        p.display_w.min(p.display_h),
        p.avg_fps.round() as u32,
        p.video_codec.to_uppercase()
    )
}
pub(crate) fn video_strategy(
    p: &cia_video_plan::VideoPlan,
    encoder: cia_video_plan::EncoderKind,
) -> Strategy {
    Strategy::Video {
        encoder: format!("{encoder:?}"),
        container: format!("{:?}", p.container).to_lowercase(),
        width: p.width,
        height: p.height,
        fps: p.fps,
        video_bps: p.video_bps,
        audio_bps: p.audio_bps,
        audio_channels: p.audio_channels,
        two_pass: p.two_pass,
    }
}
