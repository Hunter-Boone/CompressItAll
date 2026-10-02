//! Office documents (DESIGN.md 3.8) through cia-office, with images recompressed by cia-image.
use super::*;
use cia_office::{ImageFormat, MediaRecompressor, Mode, OfficeOptions, OfficeOutcome, Recompressed};

pub struct Media<'a> {
    pub ctx: &'a Ctx<'a>,
}

impl MediaRecompressor for Media<'_> {
    fn recompress_image(&self, bytes: &[u8], format: ImageFormat, budget: Option<u64>, mode: Mode, cancel: &dyn Fn() -> bool) -> Result<Option<Recompressed>, String> {
        let img = cia_image::decode(bytes).map_err(|e| e.to_string())?;
        let fmt = match format { ImageFormat::Jpeg => "jpeg", ImageFormat::Png => "png" };
        let mut o = match (budget, mode) {
            (Some(b), Mode::Fit) => cia_image::ImageOptions::fit(b, &[fmt]),
            (_, Mode::SmallerSmallest) => cia_image::ImageOptions::smaller(SmallerLevel::Smallest, &[fmt]),
            _ => cia_image::ImageOptions::smaller(SmallerLevel::KeepQuality, &[fmt]),
        };
        o.allow_format_change = false;
        o.max_long_edge = self.ctx.options.max_long_edge;
        // cia-office's cancel is the engine token we handed it, so the engine's own (Sync) cancel is equivalent.
        if cancel() {
            return Err("cancelled".into());
        }
        let p = |_: f32, _: &str| {};
        match cia_image::compress(&img, bytes, &o, self.ctx.cancel, &p).map_err(|e| e.to_string())? {
            cia_image::ImageOutcome::Encoded(r) => Ok(Some(Recompressed { bytes: r.bytes, quality_label: r.label })),
            _ => Ok(None),
        }
    }
    fn recompress_video(&self, _bytes: &[u8], _ext: &str, _budget: Option<u64>, _cancel: &dyn Fn() -> bool) -> Result<Option<Recompressed>, String> {
        // Embedded video goes through the desktop FFmpeg backend in a later milestone; left as is for now.
        Ok(None)
    }
    fn recompress_audio(&self, _bytes: &[u8], _ext: &str, _budget: Option<u64>, _cancel: &dyn Fn() -> bool) -> Result<Option<Recompressed>, String> {
        Ok(None)
    }
}

pub fn run(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx, host_can_video: bool) -> PlannerOutcome {
    if budget.is_some_and(|b| (bytes.len() as u64) < b) {
        return PlannerOutcome::KeptOriginal { attempts: vec![] };
    }
    let opts = OfficeOptions { budget_bytes: budget, mode: match (budget, ctx.smaller) { (Some(_), _) => Mode::Fit, (None, Some(SmallerLevel::Smallest)) => Mode::SmallerSmallest, _ => Mode::SmallerKeepQuality }, keep_document_details: ctx.options.keep_document_details, host_can_video };
    let media = Media { ctx };
    (ctx.progress)(0.1, "Opening document");
    match cia_office::optimise(bytes, &opts, &media, ctx.cancel) {
        Ok(OfficeOutcome::Done(r)) => {
            let size = r.bytes.len() as u64;
            let verification = VerificationReport { size_ok: ctx.hard_bytes.is_none_or(|h| size < h), decodes: r.verification.ok, checks: r.verification.checks.iter().filter(|c| c.passed).map(|c| c.name.to_string()).collect(), failures: r.verification.checks.iter().filter(|c| !c.passed).map(|c| format!("{}: {}", c.name, c.detail)).collect() };
            let summary = if r.notes.is_empty() { format!("{} images re-saved", r.media_recompressed) } else { format!("{} images re-saved; {}", r.media_recompressed, r.notes.join("; ")) };
            PlannerOutcome::Encoded(Encoded { bytes: r.bytes, format: item.detail.format.clone(), extension: item.detail.format.clone(), summary, quality: Some(r.predicted_quality), attempts: vec![attempt(1, &item.id, "cia-office", serde_json::json!({"media": r.media_recompressed, "left": r.media_left}), Some(size), None, if budget.is_none_or(|b| size < b) { AttemptVerdict::Fits } else { AttemptVerdict::Over { by_bytes: size - budget.unwrap() } })], verification })
        }
        Ok(OfficeOutcome::KeptOriginal { .. }) => PlannerOutcome::KeptOriginal { attempts: vec![] },
        Ok(OfficeOutcome::Refused { smallest_bytes }) => PlannerOutcome::Refused { code: RefusalCode::BelowQualityFloor, smallest_bytes: Some(smallest_bytes), attempts: vec![] },
        Err(cia_office::OfficeError::Encrypted) => PlannerOutcome::Refused { code: RefusalCode::Encrypted, smallest_bytes: None, attempts: vec![] },
        Err(cia_office::OfficeError::Cancelled) => PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None },
        Err(cia_office::OfficeError::Corrupt(_)) | Err(cia_office::OfficeError::NotOffice) => PlannerOutcome::Failed { code: "damaged_input", message: None, closest_bytes: None },
        Err(e) => PlannerOutcome::Failed { code: "encoder_crash", message: Some(e.to_string()), closest_bytes: None },
    }
}
