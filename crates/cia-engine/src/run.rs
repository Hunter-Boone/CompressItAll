//! The job runner (DESIGN.md 3.2, 3.9, 3.10, 3.11): dispatch each item to
//! its planner, write outputs with the no-overwrite sink, verify the written
//! bytes, package when asked, emit events, build the summary and the job log.

use crate::inspect::err_code;
use crate::log::JobLog;
use crate::output::OutputDest;
use crate::plan::{allowed_for, budgets, ctx_clone, PlanRequest};
use crate::planners::{self, Ctx, Encoded, PlannerOutcome};
use crate::{CancelToken, Engine, EngineError};
use cia_core::events::{EngineEvent, EventSink};
use cia_core::*;
use std::sync::Mutex;

struct ItemResult {
    outcome: ItemOutcome,
    attempts: Vec<Attempt>,
    /// Encoded bytes kept in memory when packaging (so the archive can be built after all items ran).
    for_archive: Option<(String, Vec<u8>)>,
}

impl Engine {
    /// Run a job to completion. Blocking; the host runs it on a worker thread (native) or inside a Web Worker.
    pub fn run(&self, req: &PlanRequest, events: &dyn EventSink, cancel: &CancelToken) -> (JobSummary, JobLog) {
        let started = crate::now_ms();
        let job_id = cia_core::new_id();
        let job = Job { id: job_id.clone(), items: req.items.clone(), goal: req.goal.clone(), packaging: req.packaging, options: req.options.clone(), created_at: started };
        let mut log = JobLog { job_id: job_id.clone(), started_at_ms: started, finished_at_ms: 0, capabilities: self.caps.clone(), job, plan: None, attempts: vec![], summary: None, warnings: vec![] };
        events.emit(EngineEvent::JobState { job_id: job_id.clone(), state: JobState::Running });

        let plan = self.preview(req, cancel).ok();
        log.plan = plan.clone();
        let limit = req.goal.limit().cloned();
        let label = limit.as_ref().map(|l| l.output_label.clone()).unwrap_or_else(|| "smaller".into());
        let allowed = allowed_for(&req.goal);
        let smaller = match &req.goal { Goal::Smaller { level } => Some(*level), _ => None };
        let cancel_fn = || cancel.is_cancelled();
        let noop = |_: f32, _: &str| {};
        let base_ctx = Ctx { options: &req.options, allowed_image: allowed.image.clone(), allowed_audio: allowed.audio.clone(), hard_bytes: limit.as_ref().map(|l| l.hard_bytes), smaller, cancel: &cancel_fn, progress: &noop };

        // Up-front refusal (per-message total too big) or a cannot-fit plan: nothing is written.
        if let Some(Plan { verdict: PlanVerdict::CannotFit { refusal }, .. }) = &plan {
            let outcomes: Vec<(ItemId, ItemOutcome)> = req.items.iter().map(|i| (i.id.clone(), ItemOutcome::Refused { refusal: refusal.clone() })).collect();
            return self.finish(req, &label, outcomes, None, started, log, events, &job_id);
        }

        let (item_budgets, _) = match budgets(self, req, &base_ctx) {
            Ok(b) => b,
            Err(e) => {
                let f = Failure { code: err_code(&e).into(), message: cia_core::copy::failure_message(err_code(&e), None, None), closest_bytes: None };
                let outcomes = req.items.iter().map(|i| (i.id.clone(), ItemOutcome::Failed { failure: f.clone() })).collect();
                return self.finish(req, &label, outcomes, None, started, log, events, &job_id);
            }
        };
        let per_message = limit.as_ref().is_some_and(|l| l.scope == LimitScope::PerMessage);
        let packaging = match &plan { Some(p) => p.packaging.clone(), None => PackagingPlan::SeparateFiles };
        let archive = matches!(packaging, PackagingPlan::Archive { .. });
        // Output directory: folder jobs get a sibling folder; archives and singles go next to the source.
        let out_dir_for = |item: &InputItem| -> String {
            let base = match &req.options.output_dir { Some(OutputDir::Folder { path }) => path.clone(), _ => self.reader.parent_dir(&item.source).unwrap_or_default() };
            base
        };
        let folder_out: Mutex<Option<String>> = Mutex::new(None);
        let folder_name = req.items.iter().find_map(|i| i.folder.clone());

        let results: Vec<ItemResult> = {
            let work = |(idx, item): (usize, &InputItem)| -> ItemResult {
                let budget = item_budgets[idx];
                let events_ref = events;
                let jid = job_id.clone();
                let iid = item.id.clone();
                let progress = move |f: f32, l: &str| { events_ref.emit(EngineEvent::Progress { job_id: jid.clone(), item_id: iid.clone(), fraction: f.clamp(0.0, 0.99), eta_ms: None, label: l.to_string() }); };
                let hard = limit.as_ref().map(|l| l.hard_for(item.kind));
                let ctx = Ctx { hard_bytes: hard, progress: &progress, ..ctx_clone(&base_ctx) };
                events.emit(EngineEvent::ItemState { job_id: job_id.clone(), item_id: item.id.clone(), state: ItemState::Encoding { attempt: 1 } });
                if cancel.is_cancelled() {
                    return ItemResult { outcome: ItemOutcome::Cancelled, attempts: vec![], for_archive: None };
                }
                let outcome = self.run_item(item, budget, &ctx, &allowed, cancel);
                let (outcome, attempts) = match outcome {
                    Err(e) => {
                        let code = err_code(&e);
                        if code == "cancelled" { (ItemOutcome::Cancelled, vec![]) } else { (ItemOutcome::Failed { failure: Failure { code: code.into(), message: cia_core::copy::failure_message(code, None, None), closest_bytes: None } }, vec![]) }
                    }
                    Ok(PlannerOutcome::Failed { code, message, closest_bytes }) => {
                        if code == "cancelled" { (ItemOutcome::Cancelled, vec![]) } else { (ItemOutcome::Failed { failure: Failure { code: code.into(), message: message.filter(|_| false).unwrap_or_else(|| cia_core::copy::failure_message(code, closest_bytes, hard)), closest_bytes } }, vec![]) }
                    }
                    Ok(PlannerOutcome::Refused { code, smallest_bytes, attempts }) => {
                        let message = cia_core::copy::refusal_message(&code, hard, smallest_bytes, item.kind);
                        let suggestions = match &code { RefusalCode::TooLongForLimit { max_duration_ms } => vec![Suggestion::Trim { max_duration_ms: *max_duration_ms }], RefusalCode::NeedsFfmpeg => vec![Suggestion::InstallFfmpeg], _ => vec![] };
                        (ItemOutcome::Refused { refusal: Refusal { code, message, smallest_bytes, suggestions } }, attempts)
                    }
                    Ok(PlannerOutcome::KeptOriginal { attempts }) => {
                        // Single inputs produce no file; inside folder/archive jobs the original is copied so the set is complete.
                        if archive || folder_name.is_some() {
                            match self.reader.read(&item.source) {
                                Ok(bytes) => {
                                    if archive {
                                        let art = Artifact { id: cia_core::new_id(), item_id: item.id.clone(), location: OutputLocation::Opfs { path: item.rel_path.clone() }, file_name: item.file_name().to_string(), bytes: bytes.len() as u64, format: item.detail.format.clone(), summary: "unchanged".into(), quality: Some(QualityLabel::Great), verification: VerificationReport { size_ok: true, decodes: true, checks: vec!["original kept".into()], failures: vec![] } };
                                        return ItemResult { outcome: ItemOutcome::KeptOriginal { artifact: art }, attempts, for_archive: Some((item.rel_path.clone(), bytes)) };
                                    }
                                    let dir = self.folder_dir(&folder_out, folder_name.as_deref(), &out_dir_for(item), &label);
                                    let sub = sub_dir(&item.rel_path);
                                    let dir = if sub.is_empty() { dir } else { self.sink.make_dir(&dir, &sub).unwrap_or(dir) };
                                    let name = cia_core::naming::first_free_name(&cia_core::naming::split_stem(item.file_name()).0, &label, cia_core::naming::split_stem(item.file_name()).1.as_deref(), |n| self.sink.exists(&dir, n));
                                    match self.sink.copy_in(&bytes, &OutputDest { dir: dir.clone(), file_name: name.clone() }) {
                                        Ok(location) => (ItemOutcome::KeptOriginal { artifact: Artifact { id: cia_core::new_id(), item_id: item.id.clone(), location, file_name: name, bytes: bytes.len() as u64, format: item.detail.format.clone(), summary: "unchanged".into(), quality: Some(QualityLabel::Great), verification: VerificationReport { size_ok: true, decodes: true, checks: vec!["original copied".into()], failures: vec![] } } }, attempts),
                                        Err(e) => (ItemOutcome::Failed { failure: Failure { code: err_code(&e).into(), message: cia_core::copy::failure_message(err_code(&e), None, None), closest_bytes: None } }, attempts),
                                    }
                                }
                                Err(e) => (ItemOutcome::Failed { failure: Failure { code: err_code(&e).into(), message: cia_core::copy::failure_message(err_code(&e), None, None), closest_bytes: None } }, attempts),
                            }
                        } else {
                            let art = Artifact { id: cia_core::new_id(), item_id: item.id.clone(), location: match &item.source { SourceRef::Path { path } => OutputLocation::Path { path: path.clone() }, SourceRef::Handle { handle_id } => OutputLocation::Opfs { path: format!("source:{handle_id}") } }, file_name: item.file_name().to_string(), bytes: item.bytes, format: item.detail.format.clone(), summary: "unchanged".into(), quality: Some(QualityLabel::Great), verification: VerificationReport { size_ok: true, decodes: true, checks: vec!["original kept".into()], failures: vec![] } };
                            (ItemOutcome::KeptOriginal { artifact: art }, attempts)
                        }
                    }
                    Ok(PlannerOutcome::Encoded(enc)) => {
                        events.emit(EngineEvent::ItemState { job_id: job_id.clone(), item_id: item.id.clone(), state: ItemState::Verifying { attempt: enc.attempts.len().max(1) as u32 } });
                        if archive {
                            // Verified again after the archive is written (3.11 item 2); keep bytes for packaging.
                            let name = entry_name(item, &enc);
                            let art = Artifact { id: cia_core::new_id(), item_id: item.id.clone(), location: OutputLocation::Opfs { path: name.clone() }, file_name: name.clone(), bytes: enc.bytes.len() as u64, format: enc.format.clone(), summary: enc.summary.clone(), quality: enc.quality, verification: enc.verification.clone() };
                            if !enc.verification.passed() {
                                return ItemResult { outcome: ItemOutcome::Failed { failure: Failure { code: "verify_failed".into(), message: cia_core::copy::failure_message("encoder_crash", None, None), closest_bytes: Some(enc.bytes.len() as u64) } }, attempts: enc.attempts, for_archive: None };
                            }
                            return ItemResult { outcome: ItemOutcome::Fitted { artifact: art }, attempts: enc.attempts.clone(), for_archive: Some((name, enc.bytes)) };
                        }
                        let (dir, name) = {
                            let base = out_dir_for(item);
                            let dir = if folder_name.is_some() { let d = self.folder_dir(&folder_out, folder_name.as_deref(), &base, &label); let sub = sub_dir(&item.rel_path); if sub.is_empty() { d } else { self.sink.make_dir(&d, &sub).unwrap_or(d) } } else { base };
                            let (stem, _) = cia_core::naming::split_stem(item.file_name());
                            let name = cia_core::naming::first_free_name(&stem, &label, Some(&enc.extension), |n| self.sink.exists(&dir, n));
                            (dir, name)
                        };
                        match self.write_and_verify(item, &enc, &dir, &name, hard) {
                            Ok(art) => (ItemOutcome::Fitted { artifact: art }, enc.attempts),
                            Err(f) => (ItemOutcome::Failed { failure: f }, enc.attempts),
                        }
                    }
                };
                ItemResult { outcome, attempts, for_archive: None }
            };
            #[cfg(not(target_arch = "wasm32"))]
            {
                use rayon::prelude::*;
                let pool = rayon::ThreadPoolBuilder::new().num_threads(self.parallelism.max(1)).build();
                match pool {
                    Ok(pool) => pool.install(|| req.items.par_iter().enumerate().map(work).collect()),
                    Err(_) => req.items.iter().enumerate().map(work).collect(),
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                req.items.iter().enumerate().map(work).collect()
            }
        };

        for (item, r) in req.items.iter().zip(results.iter()) {
            log.attempts.extend(r.attempts.iter().cloned());
            events.emit(EngineEvent::ItemDone { job_id: job_id.clone(), item_id: item.id.clone(), outcome: r.outcome.clone() });
        }
        let mut outcomes: Vec<(ItemId, ItemOutcome)> = req.items.iter().zip(results.iter()).map(|(i, r)| (i.id.clone(), r.outcome.clone())).collect();

        // Per-message jobs are all-or-nothing: any refusal/failure means nothing is kept.
        if per_message && outcomes.iter().any(|(_, o)| matches!(o, ItemOutcome::Refused { .. } | ItemOutcome::Failed { .. })) && !archive {
            for (_, o) in &outcomes {
                if let Some(a) = o.artifact() { if matches!(o, ItemOutcome::Fitted { .. }) { self.sink.remove(&a.location); } }
            }
            let refusal = outcomes.iter().find_map(|(_, o)| match o { ItemOutcome::Refused { refusal } => Some(refusal.clone()), _ => None });
            let smallest: u64 = outcomes.iter().filter_map(|(_, o)| match o { ItemOutcome::Fitted { artifact } | ItemOutcome::KeptOriginal { artifact } => Some(artifact.bytes), ItemOutcome::Refused { refusal } => refusal.smallest_bytes, ItemOutcome::Failed { failure } => failure.closest_bytes, _ => None }).sum();
            let r = refusal.unwrap_or_else(|| Refusal { code: RefusalCode::TotalTooBig, message: cia_core::copy::refusal_message(&RefusalCode::TotalTooBig, limit.as_ref().map(|l| l.hard_bytes), Some(smallest), Kind::Other), smallest_bytes: Some(smallest), suggestions: vec![] });
            outcomes = outcomes.into_iter().map(|(id, o)| (id, match o { ItemOutcome::Fitted { .. } | ItemOutcome::KeptOriginal { .. } => ItemOutcome::Refused { refusal: r.clone() }, other => other })).collect();
            return self.finish(req, &label, outcomes, None, started, log, events, &job_id);
        }

        // Packaging.
        let mut packaged: Option<Artifact> = None;
        if let PackagingPlan::Archive { format, file_name, .. } = &packaging {
            if !cancel.is_cancelled() {
                let entries: Vec<(String, Vec<u8>)> = results.iter().filter_map(|r| r.for_archive.clone()).collect();
                let all_ok = results.iter().all(|r| matches!(r.outcome, ItemOutcome::Fitted { .. } | ItemOutcome::KeptOriginal { .. }));
                if !entries.is_empty() && (all_ok || !per_message) {
                    let first = req.items.first().expect("items");
                    let dir = out_dir_for(first);
                    let (stem, ext) = cia_core::naming::split_stem(file_name);
                    let name = cia_core::naming::first_free_name(&stem.rsplit_once(" (").map(|(s, _)| s.to_string()).unwrap_or(stem.clone()), &label, ext.as_deref(), |n| self.sink.exists(&dir, n));
                    match planners::archive::package(planners::archive::kind_for(*format), &entries) {
                        Ok(bytes) => {
                            let hard = limit.as_ref().map(|l| l.hard_bytes);
                            let names: Vec<String> = entries.iter().map(|e| e.0.clone()).collect();
                            match self.sink.write(&OutputDest { dir: dir.clone(), file_name: name.clone() }, &bytes) {
                                Ok(location) => {
                                    let written = self.sink.len(&location).unwrap_or(u64::MAX);
                                    let read_back = self.sink.read(&location).unwrap_or_default();
                                    let ok = cia_archive::verify(&read_back, &names).is_ok();
                                    let size_ok = hard.is_none_or(|h| written < h);
                                    if ok && size_ok {
                                        packaged = Some(Artifact { id: cia_core::new_id(), item_id: String::new(), location, file_name: name, bytes: written, format: format!("{format:?}").to_lowercase(), summary: format!("{} files", entries.len()), quality: None, verification: VerificationReport { size_ok, decodes: ok, checks: vec!["archive re-opened, every entry read back".into(), "size read back from disk".into()], failures: vec![] } });
                                    } else {
                                        self.sink.remove(&location);
                                        let msg = if !size_ok { cia_core::copy::failure_message("over_after_retries", Some(written), hard) } else { cia_core::copy::failure_message("encoder_crash", None, None) };
                                        outcomes = outcomes.into_iter().map(|(id, o)| (id, match o { ItemOutcome::Fitted { .. } | ItemOutcome::KeptOriginal { .. } => ItemOutcome::Failed { failure: Failure { code: if size_ok { "verify_failed" } else { "over_after_retries" }.into(), message: msg.clone(), closest_bytes: Some(written) } }, other => other })).collect();
                                    }
                                }
                                Err(e) => {
                                    let code = err_code(&e);
                                    outcomes = outcomes.into_iter().map(|(id, o)| (id, match o { ItemOutcome::Fitted { .. } | ItemOutcome::KeptOriginal { .. } => ItemOutcome::Failed { failure: Failure { code: code.into(), message: cia_core::copy::failure_message(code, None, None), closest_bytes: None } }, other => other })).collect();
                                }
                            }
                        }
                        Err(m) => log.warnings.push(format!("packaging failed: {m}")),
                    }
                }
            }
        }
        self.finish(req, &label, outcomes, packaged, started, log, events, &job_id)
    }

    fn folder_dir(&self, cache: &Mutex<Option<String>>, folder: Option<&str>, base: &str, label: &str) -> String {
        let mut c = cache.lock().unwrap();
        if let Some(d) = c.as_ref() { return d.clone(); }
        let Some(folder) = folder else { return base.to_string() };
        let name = cia_core::naming::first_free_name(folder, label, None, |n| self.sink.exists(base, n));
        let d = self.sink.make_dir(base, &name).unwrap_or_else(|_| base.to_string());
        *c = Some(d.clone());
        d
    }

    fn write_and_verify(&self, item: &InputItem, enc: &Encoded, dir: &str, name: &str, hard: Option<u64>) -> Result<Artifact, Failure> {
        let location = self.sink.write(&OutputDest { dir: dir.to_string(), file_name: name.to_string() }, &enc.bytes).map_err(|e| Failure { code: err_code(&e).into(), message: cia_core::copy::failure_message(err_code(&e), None, None), closest_bytes: None })?;
        // The honesty rule: size read back from the sink, strictly under the hard limit.
        let written = self.sink.len(&location).unwrap_or(u64::MAX);
        let size_ok = hard.is_none_or(|h| written < h);
        let mut verification = enc.verification.clone();
        verification.size_ok = size_ok;
        verification.checks.push("size read back from disk".into());
        if written != enc.bytes.len() as u64 {
            verification.failures.push("written size differs from encoded size".into());
        }
        if !verification.passed() {
            self.sink.remove(&location);
            let code = if !size_ok { "over_after_retries" } else { "verify_failed" };
            return Err(Failure { code: code.into(), message: if size_ok { cia_core::copy::failure_message("encoder_crash", None, None) } else { cia_core::copy::failure_message("over_after_retries", Some(written), hard) }, closest_bytes: Some(written) });
        }
        Ok(Artifact { id: cia_core::new_id(), item_id: item.id.clone(), location, file_name: name.to_string(), bytes: written, format: enc.format.clone(), summary: enc.summary.clone(), quality: enc.quality, verification })
    }

    fn run_item(&self, item: &InputItem, budget: Option<u64>, ctx: &Ctx, allowed: &crate::plan::Allowed, cancel: &CancelToken) -> Result<PlannerOutcome, EngineError> {
        if item.detail.format == "corrupt" || item.detail.format == "unreadable" {
            return Ok(PlannerOutcome::Failed { code: "damaged_input", message: None, closest_bytes: None });
        }
        if self.needs_ffmpeg(item) {
            return Ok(PlannerOutcome::Refused { code: RefusalCode::NeedsFfmpeg, smallest_bytes: None, attempts: vec![] });
        }
        if item.kind == Kind::Video {
            return self.run_video(item, budget, ctx, allowed, cancel);
        }
        let bytes = self.reader.read(&item.source)?;
        let audio_caps = planners::audio::host_caps(&self.caps);
        Ok(match item.kind {
            Kind::Image => {
                let key = crate::plan::cache_key(item, budget, ctx);
                match self.preview_cache.lock().unwrap().remove(&key) {
                    Some(e) => PlannerOutcome::Encoded(e),
                    None => planners::image::run(item, &bytes, budget, ctx),
                }
            }
            Kind::AnimatedImage => planners::image::run_animated(item, &bytes, budget, ctx),
            Kind::Pdf => planners::pdf::run(item, &bytes, budget, ctx),
            Kind::Audio => planners::audio::run(item, &bytes, budget, &audio_caps, ctx),
            Kind::Archive => self.run_archive(item, &bytes, budget, ctx, allowed),
            Kind::OfficeDoc => planners::office::run(item, &bytes, budget, ctx, self.can_video()),
            Kind::Text => planners::plain::run_text(item, &bytes, budget, ctx),
            Kind::Video => unreachable!(),
            Kind::Other => planners::plain::run_other(item, &bytes, budget, ctx),
        })
    }

    /// Archives: expand, run each entry as a per-message sub-job against the archive's budget, repack in the same format.
    fn run_archive(&self, item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx, allowed: &crate::plan::Allowed) -> PlannerOutcome {
        if item.detail.format == "rar" {
            return PlannerOutcome::Refused { code: RefusalCode::UnsupportedInput { what: "rar".into() }, smallest_bytes: None, attempts: vec![] };
        }
        if budget.is_some_and(|b| (bytes.len() as u64) < b) {
            return PlannerOutcome::KeptOriginal { attempts: vec![] };
        }
        if !ctx.options.optimise_inside_archives {
            return planners::plain::run_other(item, bytes, budget, ctx);
        }
        let entries = match planners::archive::extract_all(bytes) {
            Ok(e) => e,
            Err(m) if m == "encrypted" => return PlannerOutcome::Refused { code: RefusalCode::Encrypted, smallest_bytes: None, attempts: vec![] },
            Err(m) => return PlannerOutcome::Refused { code: RefusalCode::UnsupportedInput { what: m }, smallest_bytes: None, attempts: vec![] },
        };
        let names: Vec<&str> = entries.iter().map(|e| e.0.as_str()).collect();
        let overhead = cia_core::zip_overhead::zip_overhead_bytes(names.iter().copied(), entries.iter().map(|e| e.1.len() as u64).max().unwrap_or(0));
        let inner_total = budget.map(|b| b.saturating_sub(overhead));
        // Allocate across entries by size share (simple proportional split; entries are usually alike).
        let total_in: u64 = entries.iter().map(|e| e.1.len() as u64).sum::<u64>().max(1);
        let mut out_entries = Vec::with_capacity(entries.len());
        let mut attempts = Vec::new();
        let audio_caps = planners::audio::host_caps(&self.caps);
        for (i, (name, data)) in entries.iter().enumerate() {
            (ctx.progress)(i as f32 / entries.len() as f32, &format!("Shrinking {}", name.rsplit('/').next().unwrap_or(name)));
            let sub_budget = inner_total.map(|t| (t as u128 * data.len() as u128 / total_in as u128) as u64);
            let det = crate::detect::detect(&data[..data.len().min(65536)], name);
            let sub_item = InputItem { id: format!("{}:{}", item.id, i), source: SourceRef::Handle { handle_id: name.clone() }, rel_path: name.clone(), bytes: data.len() as u64, kind: det.kind, detail: KindDetail { format: det.format, ..Default::default() }, folder: None };
            let r = match det.kind {
                Kind::Image => planners::image::run(&sub_item, data, sub_budget, ctx),
                Kind::AnimatedImage => planners::image::run_animated(&sub_item, data, sub_budget, ctx),
                Kind::Pdf => planners::pdf::run(&sub_item, data, sub_budget, ctx),
                Kind::Audio => planners::audio::run(&sub_item, data, sub_budget, &audio_caps, ctx),
                _ => PlannerOutcome::KeptOriginal { attempts: vec![] },
            };
            let _ = allowed;
            match r {
                PlannerOutcome::Encoded(e) => {
                    attempts.extend(e.attempts);
                    let new_name = match name.rsplit_once('.') { Some((stem, _)) => format!("{stem}.{}", e.extension), None => format!("{name}.{}", e.extension) };
                    out_entries.push((new_name, e.bytes));
                }
                PlannerOutcome::Failed { code: "cancelled", .. } => return PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None },
                _ => out_entries.push((name.clone(), data.clone())),
            }
        }
        match planners::archive::repack(&item.detail.format, &out_entries) {
            Ok((out, ext)) => {
                let size = out.len() as u64;
                if budget.is_some_and(|b| size >= b) {
                    return PlannerOutcome::Refused { code: RefusalCode::BelowQualityFloor, smallest_bytes: Some(size), attempts };
                }
                if budget.is_none() && size * 100 > (bytes.len() as u64) * 95 {
                    return PlannerOutcome::KeptOriginal { attempts };
                }
                let names: Vec<String> = out_entries.iter().map(|e| e.0.clone()).collect();
                let ok = cia_archive::verify(&out, &names).is_ok();
                PlannerOutcome::Encoded(Encoded { bytes: out, format: ext.into(), extension: ext.into(), summary: format!("{} files inside", out_entries.len()), quality: Some(QualityLabel::Good), attempts, verification: VerificationReport { size_ok: ctx.hard_bytes.is_none_or(|h| size < h), decodes: ok, checks: vec!["archive re-opened, CRCs ok".into()], failures: if ok { vec![] } else { vec!["archive did not verify".into()] } } })
            }
            Err(m) => PlannerOutcome::Failed { code: "encoder_crash", message: Some(m), closest_bytes: None },
        }
    }

    fn run_video(&self, item: &InputItem, budget: Option<u64>, ctx: &Ctx, allowed: &crate::plan::Allowed, cancel: &CancelToken) -> Result<PlannerOutcome, EngineError> {
        let video = self.video.as_ref().ok_or_else(|| EngineError::Unsupported("video".into()))?;
        let probe = video.probe(&item.source).map_err(|_| EngineError::Damaged("video".into()))?;
        let target = crate::plan::video_target(&probe, ctx, allowed);
        let popts = crate::plan::plan_options(item, ctx);
        let budget_s = budget.map(|b| { let hard = ctx.hard_bytes.unwrap_or(b); cia_video_plan::Budget { raw_budget_bytes: b, hard_bytes: hard, safety_bytes: hard.saturating_sub(b) } });
        let base = match &ctx.options.output_dir { Some(OutputDir::Folder { path }) => path.clone(), _ => self.reader.parent_dir(&item.source).unwrap_or_default() };
        let label = ctx.hard_bytes.map(|_| ()).map(|_| String::new());
        let _ = label;
        let dest = OutputDest { dir: base, file_name: String::new() };
        let progress = |f: f32, _eta: Option<u64>| (ctx.progress)(f, "Encoding");
        match video.transcode(&item.source, &probe, budget_s, target, &popts, ctx.options.video.faster, &dest, &progress, cancel) {
            Ok(r) => {
                if r.kept_original {
                    return Ok(PlannerOutcome::KeptOriginal { attempts: r.attempts });
                }
                // The backend already wrote the file; hand back the bytes through the sink so the common path verifies and names it.
                let bytes = self.sink.read(&r.location)?;
                self.sink.remove(&r.location);
                let plan = r.plan.ok_or_else(|| EngineError::Other("no plan".into()))?;
                let ext = match plan.container { cia_video_plan::Container::Mp4 => "mp4", cia_video_plan::Container::Webm => "webm" };
                Ok(PlannerOutcome::Encoded(Encoded { bytes, format: ext.into(), extension: ext.into(), summary: crate::plan::video_summary(&plan), quality: Some(plan.quality), attempts: r.attempts, verification: r.verification }))
            }
            Err(EngineError::Cancelled) => Ok(PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None }),
            Err(EngineError::Damaged(_)) => Ok(PlannerOutcome::Failed { code: "damaged_video", message: None, closest_bytes: None }),
            Err(EngineError::Other(m)) if m.starts_with("refused:") => {
                let max_ms: u64 = m.trim_start_matches("refused:").parse().unwrap_or(0);
                Ok(PlannerOutcome::Refused { code: RefusalCode::TooLongForLimit { max_duration_ms: max_ms }, smallest_bytes: None, attempts: vec![] })
            }
            Err(EngineError::Other(m)) if m.starts_with("over:") => Ok(PlannerOutcome::Failed { code: "over_after_retries", message: None, closest_bytes: m.trim_start_matches("over:").parse().ok() }),
            Err(e) => Ok(PlannerOutcome::Failed { code: "encoder_stalled", message: Some(e.to_string()), closest_bytes: None }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(&self, req: &PlanRequest, label: &str, outcomes: Vec<(ItemId, ItemOutcome)>, packaged: Option<Artifact>, started: u64, mut log: JobLog, events: &dyn EventSink, job_id: &str) -> (JobSummary, JobLog) {
        let fitted = outcomes.iter().filter(|(_, o)| matches!(o, ItemOutcome::Fitted { .. } | ItemOutcome::KeptOriginal { .. })).count();
        let cancelled = outcomes.iter().any(|(_, o)| matches!(o, ItemOutcome::Cancelled));
        let verdict = if cancelled { JobVerdict::Cancelled } else if fitted == outcomes.len() && !outcomes.is_empty() { JobVerdict::AllFit } else if fitted == 0 { JobVerdict::NoneFit } else { JobVerdict::SomeFit };
        let total_bytes = match &packaged { Some(a) => a.bytes, None => outcomes.iter().filter_map(|(_, o)| o.artifact().map(|a| a.bytes)).sum() };
        let input_bytes = req.items.iter().map(|i| i.bytes).sum();
        let smallest = outcomes.iter().filter_map(|(_, o)| match o { ItemOutcome::Refused { refusal } => refusal.smallest_bytes, ItemOutcome::Failed { failure } => failure.closest_bytes, _ => None }).max();
        let headline = cia_core::copy::result_headline(verdict, label, fitted, outcomes.len(), smallest, req.goal.limit().map(|l| l.hard_bytes), if cancelled { fitted } else { 0 });
        let finished = crate::now_ms();
        let summary = JobSummary { job_id: job_id.to_string(), outcomes, packaged, input_bytes, total_bytes, verdict, headline, elapsed_ms: finished.saturating_sub(started) };
        log.finished_at_ms = finished;
        log.summary = Some(summary.clone());
        events.emit(EngineEvent::JobDone { job_id: job_id.to_string(), summary: summary.clone() });
        (summary, log)
    }
}

fn sub_dir(rel_path: &str) -> String {
    let parts: Vec<&str> = rel_path.split(['/', '\\']).collect();
    if parts.len() <= 2 { String::new() } else { parts[1..parts.len() - 1].join("/") }
}

fn entry_name(item: &InputItem, enc: &Encoded) -> String {
    let rel = item.rel_path.replace('\\', "/");
    let rel = rel.split_once('/').map(|(_, rest)| rest.to_string()).unwrap_or(rel.clone());
    match rel.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.{}", enc.extension),
        None => format!("{rel}.{}", enc.extension),
    }
}
