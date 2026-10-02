use super::*;

/// mozjpeg as the PDF image re-encoder.
pub struct MozJpeg;
impl cia_pdf::JpegEncoder for MozJpeg {
    fn encode_rgb(&self, rgb: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String> {
        cia_mozjpeg::encode(rgb, w, h, cia_mozjpeg::PixelLayout::Rgb, &cia_mozjpeg::JpegOptions::photo(quality)).map_err(|e| e.to_string())
    }
    fn encode_gray(&self, gray: &[u8], w: u32, h: u32, quality: u8) -> Result<Vec<u8>, String> {
        cia_mozjpeg::encode(gray, w, h, cia_mozjpeg::PixelLayout::Gray, &cia_mozjpeg::JpegOptions::photo(quality)).map_err(|e| e.to_string())
    }
}

pub fn run(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx) -> PlannerOutcome {
    let opts = cia_pdf::PdfOptions { keep_document_details: ctx.options.keep_document_details, budget_bytes: budget, smaller_mode: if budget.is_none() { Some(ctx.smaller.unwrap_or(SmallerLevel::KeepQuality)) } else { None } };
    match cia_pdf::optimise(bytes, &opts, &MozJpeg, ctx.cancel) {
        Ok(cia_pdf::PdfOutcome::Done(r)) => {
            let attempts = r.attempts.iter().enumerate().map(|(i, a)| attempt(i as u32 + 1, &item.id, "lopdf+mozjpeg", serde_json::json!({"step": a.step, "quality": a.quality, "cap": a.cap}), Some(a.bytes), None, match budget { Some(b) if a.bytes >= b => AttemptVerdict::Over { by_bytes: a.bytes - b }, _ => AttemptVerdict::Fits })).collect();
            let pages = item.detail.page_count.unwrap_or(r.verification.page_count);
            let quality = Some(match r.quality { None => QualityLabel::Great, Some(q) if q >= 75 => QualityLabel::Good, _ => QualityLabel::Okay });
            let verification = VerificationReport { size_ok: ctx.hard_bytes.is_none_or(|h| (r.bytes.len() as u64) < h), decodes: r.verification.ok, checks: r.verification.checks.iter().filter(|c| c.passed).map(|c| c.name.to_string()).collect(), failures: r.verification.checks.iter().filter(|c| !c.passed).map(|c| format!("{}: {}", c.name, c.detail)).collect() };
            PlannerOutcome::Encoded(Encoded { bytes: r.bytes, format: "pdf".into(), extension: "pdf".into(), summary: format!("{pages} pages{}", if r.images_recompressed > 0 { format!(", {} images re-saved", r.images_recompressed) } else { String::new() }), quality, attempts, verification })
        }
        Ok(cia_pdf::PdfOutcome::KeptOriginal { .. }) => PlannerOutcome::KeptOriginal { attempts: vec![] },
        Ok(cia_pdf::PdfOutcome::Refused { smallest_bytes }) => PlannerOutcome::Refused { code: RefusalCode::BelowQualityFloor, smallest_bytes: Some(smallest_bytes), attempts: vec![] },
        Ok(cia_pdf::PdfOutcome::Encrypted) => PlannerOutcome::Refused { code: RefusalCode::Encrypted, smallest_bytes: None, attempts: vec![] },
        Err(cia_pdf::PdfError::Cancelled) => PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None },
        Err(cia_pdf::PdfError::Damaged(_)) => PlannerOutcome::Failed { code: "damaged_pdf", message: None, closest_bytes: None },
        Err(e) => PlannerOutcome::Failed { code: "encoder_crash", message: Some(e.to_string()), closest_bytes: None },
    }
}
