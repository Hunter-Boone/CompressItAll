use super::*;
use cia_audio::plan::{AudioFormat, HostAudioCaps};

pub fn host_caps(caps: &Capabilities) -> HostAudioCaps {
    HostAudioCaps { mp3: caps.mp3_encode, aac: caps.aac_encode, opus: caps.opus_encode, flac: true, wav: true }
}

fn allowed<'a>(ctx: &'a Ctx) -> Vec<&'a str> {
    match ctx.options.audio.format {
        AudioFormatPref::Automatic => ctx.allowed_audio.iter().map(String::as_str).collect(),
        AudioFormatPref::Mp3 => vec!["mp3"],
        AudioFormatPref::Aac => vec!["m4a_aac"],
        AudioFormatPref::Opus => vec!["ogg_opus"],
        AudioFormatPref::Flac => vec!["flac"],
    }
}

/// Predicted size for the preview (no encode).
pub fn predict(info: &cia_audio::AudioInfo, source_bytes: u64, budget: Option<u64>, caps: &HostAudioCaps, ctx: &Ctx) -> Result<(u64, String, QualityLabel), RefusalCode> {
    let formats = allowed(ctx);
    match budget {
        Some(b) => {
            let plan = cia_audio::plan::plan_fit(info.duration_ms, b, &formats, caps, info.channels, if info.lossless { Some(source_bytes) } else { None })?;
            Ok((plan.predicted_bytes, describe(&plan), label(&plan)))
        }
        None => Ok(match cia_audio::plan::plan_smaller(info, ctx.smaller.unwrap_or(SmallerLevel::KeepQuality)) {
            cia_audio::plan::SmallerDecision::ToFlac => (source_bytes / 2, "FLAC".into(), QualityLabel::Great),
            cia_audio::plan::SmallerDecision::ReencodeFlac => (source_bytes * 97 / 100, "FLAC".into(), QualityLabel::Great),
            cia_audio::plan::SmallerDecision::KeepOriginal => (source_bytes, info.codec.to_uppercase(), QualityLabel::Great),
            cia_audio::plan::SmallerDecision::ToOpus { bps, channels } => (cia_audio::plan::predict_bytes(AudioFormat::OggOpus, bps, info.duration_ms), format!("Opus, {} kb/s", bps / 1000), if channels > 1 { QualityLabel::Good } else { QualityLabel::Okay }),
        }),
    }
}

fn describe(p: &cia_audio::plan::AudioPlan) -> String {
    if p.format.is_lossless() { p.format.id().to_uppercase().replace("M4A_", "") } else { format!("{}, {} kb/s", pretty(p.format), p.bitrate_bps / 1000) }
}
fn pretty(f: AudioFormat) -> &'static str {
    match f.id() { "mp3" => "MP3", "m4a_aac" => "AAC", "ogg_opus" => "Opus", "flac" => "FLAC", _ => "WAV" }
}
fn label(p: &cia_audio::plan::AudioPlan) -> QualityLabel {
    if p.format.is_lossless() { QualityLabel::Great } else if p.bitrate_bps >= 96_000 { QualityLabel::Good } else { QualityLabel::Okay }
}

/// Encode with the pure-Rust encoders this crate has (FLAC, WAV, Opus natively). MP3/AAC need the video backend (FFmpeg); the engine routes those.
pub fn run(item: &InputItem, bytes: &[u8], budget: Option<u64>, caps: &HostAudioCaps, ctx: &Ctx) -> PlannerOutcome {
    let info = match cia_audio::probe(bytes) {
        Ok(i) => i,
        Err(_) => return PlannerOutcome::Failed { code: "damaged_input", message: None, closest_bytes: None },
    };
    if info.needs_ffmpeg {
        return PlannerOutcome::Refused { code: RefusalCode::NeedsFfmpeg, smallest_bytes: None, attempts: vec![] };
    }
    let formats = allowed(ctx);
    let mut attempts = Vec::new();
    let decoded = match cia_audio::decode(bytes, None) {
        Ok(d) => d,
        Err(e) => return PlannerOutcome::Failed { code: "damaged_input", message: Some(e.to_string()), closest_bytes: None },
    };
    let mut plan = match budget {
        Some(b) => {
            if (bytes.len() as u64) < b && !ctx.options.audio.format.ne(&AudioFormatPref::Automatic) && formats.iter().any(|f| AudioFormat::from_id(f).is_some_and(|af| af.id() == native_id(&info.codec))) {
                return PlannerOutcome::KeptOriginal { attempts: vec![] };
            }
            match cia_audio::plan::plan_fit(info.duration_ms, b, &formats, caps, info.channels, if info.lossless { Some(bytes.len() as u64) } else { None }) {
                Ok(p) => p,
                Err(code) => return PlannerOutcome::Refused { code, smallest_bytes: None, attempts },
            }
        }
        None => match cia_audio::plan::plan_smaller(&info, ctx.smaller.unwrap_or(SmallerLevel::KeepQuality)) {
            cia_audio::plan::SmallerDecision::KeepOriginal => return PlannerOutcome::KeptOriginal { attempts: vec![] },
            cia_audio::plan::SmallerDecision::ToFlac | cia_audio::plan::SmallerDecision::ReencodeFlac => cia_audio::plan::AudioPlan { format: AudioFormat::Flac, bitrate_bps: 0, channels: info.channels, sample_rate: info.sample_rate, predicted_bytes: bytes.len() as u64 / 2, attempt: 1 },
            cia_audio::plan::SmallerDecision::ToOpus { bps, channels } => cia_audio::plan::AudioPlan { format: AudioFormat::OggOpus, bitrate_bps: bps, channels, sample_rate: 48_000, predicted_bytes: cia_audio::plan::predict_bytes(AudioFormat::OggOpus, bps, info.duration_ms), attempt: 1 },
        },
    };
    let hard = ctx.hard_bytes;
    for n in 1..=4u32 {
        if (ctx.cancel)() {
            return PlannerOutcome::Failed { code: "cancelled", message: None, closest_bytes: None };
        }
        (ctx.progress)(0.2 * n as f32, "Encoding audio");
        let src = if plan.channels < decoded.channels { cia_audio::downmix_mono(&decoded) } else { decoded.clone() };
        let out: Result<Vec<u8>, String> = match plan.format {
            AudioFormat::Flac => cia_audio::encode_flac(&src, 8).map_err(|e| e.to_string()),
            AudioFormat::Wav => Ok(cia_audio::encode_wav(&src)),
            AudioFormat::OggOpus => {
                #[cfg(feature = "opus")]
                {
                    cia_audio::encode_opus_ogg(&src, plan.bitrate_bps, plan.channels, if plan.opus_voip() { cia_audio::OpusApplication::Voip } else { cia_audio::OpusApplication::Audio }).map_err(|e| e.to_string())
                }
                #[cfg(not(feature = "opus"))]
                {
                    Err("opus encoder not available on this host".to_string())
                }
            }
            AudioFormat::Mp3 | AudioFormat::M4aAac => Err("needs ffmpeg".into()),
        };
        let bytes_out = match out {
            Ok(b) => b,
            Err(m) if m == "needs ffmpeg" => return PlannerOutcome::Refused { code: RefusalCode::NeedsFfmpeg, smallest_bytes: None, attempts },
            Err(m) => return PlannerOutcome::Failed { code: "encoder_crash", message: Some(m), closest_bytes: None },
        };
        let size = bytes_out.len() as u64;
        let fits = budget.is_none_or(|b| size < b) && hard.is_none_or(|h| size < h);
        attempts.push(attempt(n, &item.id, plan.format.id(), serde_json::json!({"bitrate": plan.bitrate_bps, "channels": plan.channels, "sample_rate": plan.sample_rate}), Some(size), None, if fits { AttemptVerdict::Fits } else { AttemptVerdict::Over { by_bytes: size.saturating_sub(budget.unwrap_or(size)) } }));
        if fits {
            if budget.is_none() && size * 100 > (bytes.len() as u64) * 97 && plan.format.is_lossless() {
                return PlannerOutcome::KeptOriginal { attempts };
            }
            let v = cia_audio::verify(&bytes_out, info.duration_ms, plan.channels, hard);
            let verification = VerificationReport { size_ok: v.size_ok, decodes: v.decodes, checks: vec!["full decode".into(), "duration within 0.1 s".into()], failures: v.problems.clone() };
            if !verification.passed() {
                return PlannerOutcome::Failed { code: "encoder_crash", message: Some(v.problems.join("; ")), closest_bytes: Some(size) };
            }
            return PlannerOutcome::Encoded(Encoded { bytes: bytes_out, format: plan.format.id().into(), extension: plan.format.extension().into(), summary: describe(&plan), quality: Some(label(&plan)), attempts, verification });
        }
        match cia_audio::plan::retry_scale(&plan, size, budget.unwrap_or(size)) {
            Some(p) => plan = p,
            None => return PlannerOutcome::Failed { code: "over_after_retries", message: None, closest_bytes: Some(size) },
        }
    }
    PlannerOutcome::Failed { code: "over_after_retries", message: None, closest_bytes: attempts.last().and_then(|a| a.output_bytes) }
}

fn native_id(codec: &str) -> &'static str {
    match codec {
        "mp3" => "mp3",
        "flac" => "flac",
        "opus" => "ogg_opus",
        "aac" => "m4a_aac",
        "pcm" => "wav",
        _ => "other",
    }
}
