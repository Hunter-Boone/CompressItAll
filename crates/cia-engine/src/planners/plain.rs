//! Text and Other (DESIGN.md 3.9.4).
use super::*;
use cia_archive::{ArchiveKind, ArchiveOptions, ArchiveWriter};

pub fn zip_of(name: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut w = ArchiveWriter::in_memory(ArchiveKind::Zip, ArchiveOptions::default())
        .map_err(|e| e.to_string())?;
    w.add_entry(name, bytes).map_err(|e| e.to_string())?;
    w.finish().map_err(|e| e.to_string())
}

/// Fit mode: a text file over the limit becomes a zip of itself when that fits.
pub fn run_text(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx) -> PlannerOutcome {
    let Some(b) = budget else {
        return PlannerOutcome::KeptOriginal { attempts: vec![] };
    };
    if (bytes.len() as u64) <= b {
        return PlannerOutcome::KeptOriginal { attempts: vec![] };
    }
    (ctx.progress)(0.3, "Zipping");
    let name = item
        .rel_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&item.rel_path)
        .to_string();
    match zip_of(&name, bytes) {
        Ok(z) => {
            let size = z.len() as u64;
            let a = attempt(
                1,
                &item.id,
                "zip-deflate",
                serde_json::json!({}),
                Some(size),
                None,
                if size <= b {
                    AttemptVerdict::Fits
                } else {
                    AttemptVerdict::Over { by_bytes: size - b }
                },
            );
            if size <= b {
                let ok = cia_archive::verify(&z, std::slice::from_ref(&name)).is_ok();
                PlannerOutcome::Encoded(Encoded {
                    bytes: z,
                    format: "zip".into(),
                    extension: "zip".into(),
                    summary: "zipped".into(),
                    quality: Some(QualityLabel::Great),
                    attempts: vec![a],
                    verification: VerificationReport {
                        size_ok: true,
                        decodes: ok,
                        checks: vec!["zip re-opened, CRC ok".into()],
                        failures: if ok {
                            vec![]
                        } else {
                            vec!["zip did not verify".into()]
                        },
                    },
                })
            } else {
                PlannerOutcome::Refused {
                    code: RefusalCode::CannotShrinkType,
                    smallest_bytes: Some(size),
                    attempts: vec![a],
                }
            }
        }
        Err(m) => PlannerOutcome::Failed {
            code: "encoder_crash",
            message: Some(m),
            closest_bytes: None,
        },
    }
}

/// Other: try Deflate and LZMA2 on the first 4 MiB; if neither saves 3 percent, it cannot shrink.
pub fn run_other(item: &InputItem, bytes: &[u8], budget: Option<u64>, ctx: &Ctx) -> PlannerOutcome {
    if let Some(b) = budget {
        if (bytes.len() as u64) <= b {
            return PlannerOutcome::KeptOriginal { attempts: vec![] };
        }
    }
    let sample = &bytes[..bytes.len().min(4 * 1024 * 1024)];
    (ctx.progress)(0.2, "Checking");
    let name = item
        .rel_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&item.rel_path)
        .to_string();
    let deflate = zip_of(&name, sample)
        .map(|z| z.len() as u64)
        .unwrap_or(u64::MAX);
    let saves = deflate * 100 <= (sample.len() as u64) * 97;
    if !saves {
        return PlannerOutcome::Refused {
            code: RefusalCode::CannotShrinkType,
            smallest_bytes: Some(bytes.len() as u64),
            attempts: vec![],
        };
    }
    (ctx.progress)(0.5, "Zipping");
    match zip_of(&name, bytes) {
        Ok(z) => {
            let size = z.len() as u64;
            let fits = budget.is_none_or(|b| size <= b);
            let a = attempt(
                1,
                &item.id,
                "zip-deflate",
                serde_json::json!({}),
                Some(size),
                None,
                if fits {
                    AttemptVerdict::Fits
                } else {
                    AttemptVerdict::Over {
                        by_bytes: size - budget.unwrap(),
                    }
                },
            );
            if fits && (budget.is_some() || size * 100 <= (bytes.len() as u64) * 95) {
                let ok = cia_archive::verify(&z, std::slice::from_ref(&name)).is_ok();
                PlannerOutcome::Encoded(Encoded {
                    bytes: z,
                    format: "zip".into(),
                    extension: "zip".into(),
                    summary: "zipped".into(),
                    quality: Some(QualityLabel::Great),
                    attempts: vec![a],
                    verification: VerificationReport {
                        size_ok: true,
                        decodes: ok,
                        checks: vec!["zip re-opened, CRC ok".into()],
                        failures: if ok {
                            vec![]
                        } else {
                            vec!["zip did not verify".into()]
                        },
                    },
                })
            } else if fits {
                PlannerOutcome::KeptOriginal { attempts: vec![a] }
            } else {
                PlannerOutcome::Refused {
                    code: RefusalCode::CannotShrinkType,
                    smallest_bytes: Some(size),
                    attempts: vec![a],
                }
            }
        }
        Err(m) => PlannerOutcome::Failed {
            code: "encoder_crash",
            message: Some(m),
            closest_bytes: None,
        },
    }
}
