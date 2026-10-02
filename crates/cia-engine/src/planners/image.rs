use super::*;
use cia_image::{ImageOptions, ImageOutcome};

fn opts(ctx: &Ctx, budget: Option<u64>) -> ImageOptions {
    let allowed: Vec<&str> = ctx.allowed_image.iter().map(String::as_str).collect();
    let mut o = match (budget, ctx.smaller) {
        (Some(b), _) => ImageOptions::fit(b, &allowed),
        (None, Some(level)) => ImageOptions::smaller(level, &allowed),
        (None, None) => ImageOptions::smaller(SmallerLevel::KeepQuality, &allowed),
    };
    o.allow_format_change = ctx.options.allow_format_change && budget.is_some();
    o.modern_formats = ctx.options.modern_formats;
    o.flatten_transparency = ctx.options.flatten_transparency;
    o.max_long_edge = ctx.options.max_long_edge;
    o.keep_photo_details = ctx.options.keep_photo_details;
    o.keep_location = ctx.options.keep_photo_details && ctx.options.keep_location;
    o
}

/// Pass 0: lossless and floor sizes (real encodes, so previews for images are exact).
pub fn sizes(bytes: &[u8], ctx: &Ctx) -> Result<(Sizes, cia_image::DecodedImage), cia_image::ImageError> {
    let img = cia_image::decode(bytes)?;
    let o = opts(ctx, Some(u64::MAX / 2));
    let lossless = cia_image::lossless_size(&img, &o, ctx.cancel)?.min(bytes.len() as u64);
    let floor = cia_image::floor_size(&img, &o, ctx.cancel)?.min(lossless);
    Ok((Sizes { lossless, floor }, img))
}

pub fn run(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx) -> PlannerOutcome {
    let t0 = std::time::Instant::now();
    let img = match cia_image::decode(bytes) {
        Ok(i) => i,
        Err(cia_image::ImageError::Unsupported(what)) => return PlannerOutcome::Refused { code: RefusalCode::UnsupportedInput { what }, smallest_bytes: None, attempts: vec![] },
        Err(_) => return PlannerOutcome::Failed { code: "damaged_input", message: None, closest_bytes: None },
    };
    log::debug!("decode {} ms", t0.elapsed().as_millis());
    let o = opts(ctx, budget);
    match cia_image::compress(&img, bytes, &o, ctx.cancel, ctx.progress) {
        Ok(ImageOutcome::Encoded(r)) => {
            let attempts = to_attempts(&item.id, &r.attempts, budget);
            let verification = cia_image::verify(&r.bytes, &cia_image::Expect { format: r.format, width: r.width, height: r.height, has_alpha: img.has_alpha && !o.flatten_transparency, gps_allowed: o.keep_location, hard_bytes: ctx.hard_bytes });
            PlannerOutcome::Encoded(Encoded { bytes: r.bytes, format: r.format.token().into(), extension: r.format.extension().into(), summary: format!("{} × {} {}", r.width, r.height, r.format.token().to_uppercase()), quality: Some(r.label), attempts, verification })
        }
        Ok(ImageOutcome::KeptOriginal { attempts }) => PlannerOutcome::KeptOriginal { attempts: to_attempts(&item.id, &attempts, budget) },
        Ok(ImageOutcome::Refused { smallest_bytes, attempts }) => PlannerOutcome::Refused { code: RefusalCode::BelowQualityFloor, smallest_bytes: Some(smallest_bytes), attempts: to_attempts(&item.id, &attempts, budget) },
        Err(cia_image::ImageError::Cancelled) => PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None },
        Err(e) => PlannerOutcome::Failed { code: "encoder_crash", message: Some(e.to_string()), closest_bytes: None },
    }
}

fn to_attempts(id: &str, a: &[cia_image::ImageAttempt], budget: Option<u64>) -> Vec<Attempt> {
    a.iter().enumerate().map(|(i, a)| { let mut at = attempt(i as u32 + 1, id, &a.candidate.name(), serde_json::json!({"quality": a.quality, "width": a.width, "height": a.height}), Some(a.bytes), a.score, match budget { Some(b) if a.bytes >= b => AttemptVerdict::Over { by_bytes: a.bytes - b }, _ => AttemptVerdict::Fits }); at.elapsed_ms = a.elapsed_ms; at }).collect()
}

pub fn run_animated(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx) -> PlannerOutcome {
    let anim = match cia_image::animated::decode_animation(bytes) {
        Ok(a) => a,
        Err(cia_image::ImageError::Unsupported(what)) => return PlannerOutcome::Refused { code: RefusalCode::UnsupportedInput { what }, smallest_bytes: None, attempts: vec![] },
        Err(_) => return PlannerOutcome::Failed { code: "damaged_input", message: None, closest_bytes: None },
    };
    if !ctx.allowed_image.iter().any(|f| f == "gif") {
        return PlannerOutcome::Refused { code: RefusalCode::CannotShrinkType, smallest_bytes: Some(bytes.len() as u64), attempts: vec![] };
    }
    if budget.is_some_and(|b| (bytes.len() as u64) < b) {
        return PlannerOutcome::KeptOriginal { attempts: vec![] };
    }
    match cia_image::animated::compress_animation(&anim, &cia_image::animated::AnimOptions { budget_bytes: budget }, ctx.cancel, ctx.progress) {
        Ok(cia_image::animated::AnimOutcome::Encoded(r)) => {
            let ok = cia_image::animated::decode_animation(&r.bytes).map(|b| b.frames.len() == r.frames).unwrap_or(false);
            let size_ok = ctx.hard_bytes.is_none_or(|h| (r.bytes.len() as u64) < h);
            PlannerOutcome::Encoded(Encoded { summary: format!("{} × {} GIF, {} frames", r.width, r.height, r.frames), quality: Some(r.label), attempts: vec![attempt(1, &item.id, "gif", serde_json::json!({"colours": r.colours, "frame_dropped": r.frame_dropped}), Some(r.bytes.len() as u64), None, AttemptVerdict::Fits)], verification: VerificationReport { size_ok, decodes: ok, checks: vec!["frame count as planned".into()], failures: if ok { vec![] } else { vec!["frame count differs".into()] } }, bytes: r.bytes, format: "gif".into(), extension: "gif".into() })
        }
        Ok(cia_image::animated::AnimOutcome::KeptOriginal) => PlannerOutcome::KeptOriginal { attempts: vec![] },
        Ok(cia_image::animated::AnimOutcome::Refused { smallest_bytes }) => PlannerOutcome::Refused { code: RefusalCode::BelowQualityFloor, smallest_bytes: Some(smallest_bytes), attempts: vec![] },
        Err(cia_image::ImageError::Cancelled) => PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None },
        Err(e) => PlannerOutcome::Failed { code: "encoder_crash", message: Some(e.to_string()), closest_bytes: None },
    }
}
