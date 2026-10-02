//! `cia matrix`: every fixture against every preset (DESIGN.md 7.3). A row
//! passes only when the written file is re-verified independently of the
//! engine; any `Fitted` row at or over the limit is an honesty violation (exit 2).
use crate::{engine, Printer};
use anyhow::Result;
use cia_core::*;
use cia_engine::inspect::InputSpec;
use cia_engine::{CancelToken, PlanRequest};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Row {
    fixture: String,
    preset: String,
    outcome: String,
    input_bytes: u64,
    output_bytes: Option<u64>,
    limit_bytes: Option<u64>,
    summary: String,
    quality: Option<QualityLabel>,
    elapsed_ms: u64,
    pass: bool,
    honesty_violation: bool,
    note: String,
}

const SMOKE_PRESETS: &[&str] = &["discord-free", "email", "whatsapp", "smaller"];

pub fn run(
    cli: &crate::Cli,
    fixtures: &Path,
    presets: &str,
    out: &Path,
    smoke: bool,
    only: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let preset_ids: Vec<String> = match presets {
        "all" => cia_core::presets::all()
            .iter()
            .filter(|p| p.mode != cia_core::presets::PresetMode::Custom)
            .map(|p| p.id.clone())
            .collect(),
        "smoke" => SMOKE_PRESETS.iter().map(|s| s.to_string()).collect(),
        list => list.split(',').map(|s| s.trim().to_string()).collect(),
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixtures)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != "manifest.json"))
        .collect();
    files.sort();
    if smoke {
        files.retain(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            let m = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            m < 80_000_000
                && !n.contains("15min")
                && !n.contains("70min")
                && !n.contains("2min")
                && !n.contains("48mp")
        });
    }
    if let Some(o) = only {
        files.retain(|p| p.file_name().unwrap().to_string_lossy().contains(o));
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut violations = 0;
    // Whether this machine's FFmpeg has any working H.264 encoder; a NoEncoder refusal is only
    // the expected outcome when it does not.
    let has_h264 = cia_engine::video_ffmpeg::FfmpegBackend::locate(&super::app_data(cli))
        .map(|ff| !ff.report.chain(true, false).is_empty())
        .unwrap_or(false);
    for file in &files {
        for pid in &preset_ids {
            let job_id = cia_core::new_id();
            let e = engine(cli, &job_id);
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            let spec = InputSpec {
                source: SourceRef::Path {
                    path: file.to_string_lossy().to_string(),
                },
                rel_path: name.clone(),
                folder: None,
            };
            let items = e.inspect(&[spec]);
            let goal = crate::goal(pid, None, pid == "smaller")?;
            let limit = goal.limit().map(|l| l.hard_for(items[0].kind));
            let out_dir = out.join("outputs").join(pid);
            std::fs::create_dir_all(&out_dir)?;
            let options = JobOptions {
                output_dir: Some(OutputDir::Folder {
                    path: out_dir.to_string_lossy().to_string(),
                }),
                ..Default::default()
            };
            let req = PlanRequest {
                items,
                goal,
                packaging: Packaging::Auto,
                options,
            };
            eprintln!("== {name} × {pid}");
            let (summary, log) = e.run(&req, &Printer { json: true }, &CancelToken::new());
            let _ = std::fs::write(out.join(format!("{name}.{pid}.log.json")), log.to_json());
            let (_, outcome) = summary
                .outcomes
                .first()
                .cloned()
                .unwrap_or((String::new(), ItemOutcome::Cancelled));
            let (outcome_s, output_bytes, summary_s, quality, mut pass, mut note, mut violation) =
                match &outcome {
                    ItemOutcome::Fitted { artifact } => {
                        // Independent re-check: read the file from disk, compare the size to the limit, and decode with a second path.
                        let path = match &artifact.location {
                            OutputLocation::Path { path } | OutputLocation::Opfs { path } => {
                                path.clone()
                            }
                        };
                        let real = std::fs::metadata(&path).map(|m| m.len()).ok();
                        let size_ok = match (real, limit) {
                            (Some(r), Some(l)) => r < l,
                            (Some(_), None) => true,
                            _ => false,
                        };
                        let decodes = real.is_some() && independent_decode(&path);
                        (
                            "fitted".to_string(),
                            real,
                            artifact.summary.clone(),
                            artifact.quality,
                            size_ok && decodes,
                            if !decodes {
                                "output did not decode independently".into()
                            } else if !size_ok {
                                "written file at or over the limit".into()
                            } else {
                                String::new()
                            },
                            !size_ok && real.is_some(),
                        )
                    }
                    ItemOutcome::KeptOriginal { artifact } => (
                        "kept_original".into(),
                        Some(artifact.bytes),
                        artifact.summary.clone(),
                        artifact.quality,
                        limit.is_none_or(|l| artifact.bytes < l),
                        String::new(),
                        false,
                    ),
                    ItemOutcome::Refused { refusal } => (
                        "refused".into(),
                        refusal.smallest_bytes,
                        refusal.message.clone(),
                        None,
                        expected_refusal(&name, pid, &refusal.code, has_h264),
                        format!("{:?}", refusal.code),
                        false,
                    ),
                    ItemOutcome::Failed { failure } => (
                        "failed".into(),
                        failure.closest_bytes,
                        failure.message.clone(),
                        None,
                        expected_failure(&name, summary.input_bytes),
                        failure.code.clone(),
                        false,
                    ),
                    ItemOutcome::Cancelled => (
                        "cancelled".into(),
                        None,
                        String::new(),
                        None,
                        false,
                        String::new(),
                        false,
                    ),
                };
            if violation {
                violations += 1;
                pass = false;
                note = format!("HONESTY VIOLATION: {note}");
            }
            let _ = &mut violation;
            rows.push(Row {
                fixture: name,
                preset: pid.clone(),
                outcome: outcome_s,
                input_bytes: summary.input_bytes,
                output_bytes,
                limit_bytes: limit,
                summary: summary_s,
                quality,
                elapsed_ms: summary.elapsed_ms,
                pass,
                honesty_violation: violation,
                note,
            });
        }
    }
    std::fs::write(
        out.join("matrix.json"),
        serde_json::to_string_pretty(&rows)?,
    )?;
    let mut md = String::from("| Fixture | Preset | Outcome | In | Out | Limit | Quality | ms | Pass | Note |\n|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.fixture,
            r.preset,
            r.outcome,
            r.input_bytes,
            r.output_bytes.map(|b| b.to_string()).unwrap_or_default(),
            r.limit_bytes.map(|b| b.to_string()).unwrap_or_default(),
            r.quality.map(|q| q.word()).unwrap_or(""),
            r.elapsed_ms,
            if r.pass { "PASS" } else { "FAIL" },
            r.note
        ));
    }
    std::fs::write(out.join("matrix.md"), &md)?;
    let mut junit = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"cia-matrix\">\n",
    );
    for r in &rows {
        junit.push_str(&format!(
            "  <testcase classname=\"{}\" name=\"{}\" time=\"{}\">{}</testcase>\n",
            r.preset,
            r.fixture.replace('"', "'"),
            r.elapsed_ms as f64 / 1000.0,
            if r.pass {
                String::new()
            } else {
                format!("<failure message=\"{}\"/>", r.note.replace('"', "'"))
            }
        ));
    }
    junit.push_str("</testsuite>\n");
    std::fs::write(out.join("junit.xml"), junit)?;
    let passed = rows.iter().filter(|r| r.pass).count();
    println!("{md}");
    println!(
        "{passed} of {} rows pass; {violations} honesty violations",
        rows.len()
    );
    for r in rows.iter().filter(|r| r.honesty_violation) {
        eprintln!(
            "\x1b[31mHONESTY VIOLATION: {} × {}: {}\x1b[0m",
            r.fixture, r.preset, r.note
        );
    }
    if violations > 0 {
        std::process::exit(2);
    }
    if passed < rows.len() {
        std::process::exit(1);
    }
    Ok(())
}

fn independent_decode(path: &str) -> bool {
    // Second path: the `image` crate for images, header checks for the rest. ffprobe for video when available.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let det = cia_engine::detect::detect(&bytes[..bytes.len().min(65536)], path);
    match det.kind {
        Kind::Image => image::load_from_memory(&bytes).is_ok(),
        Kind::AnimatedImage => image::load_from_memory(&bytes).is_ok(),
        Kind::Pdf => {
            bytes.starts_with(b"%PDF") && bytes.windows(5).rev().take(1024).any(|w| w == b"%%EOF")
        }
        Kind::Archive | Kind::OfficeDoc => cia_archive_ok(&bytes),
        Kind::Video => ffprobe_ok(path),
        Kind::Audio => ffprobe_ok(path) || !bytes.is_empty(),
        _ => !bytes.is_empty(),
    }
}

fn cia_archive_ok(bytes: &[u8]) -> bool {
    cia_archive::open(bytes)
        .map(|mut a| a.extract_all().is_ok())
        .unwrap_or(false)
}

fn ffprobe_ok(path: &str) -> bool {
    let probe = std::env::var_os("CIA_FFMPEG")
        .map(|p| {
            let p = PathBuf::from(p);
            if p.is_dir() {
                p.join("ffprobe")
            } else {
                p.with_file_name("ffprobe")
            }
        })
        .unwrap_or_else(|| PathBuf::from("ffprobe"));
    std::process::Command::new(probe)
        .args(["-v", "error", "-show_format", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true)
}

/// Refusals that the fixture set expects (the design's expectations.toml, inline for now).
fn expected_refusal(name: &str, preset: &str, code: &RefusalCode, has_h264: bool) -> bool {
    let n = name.to_ascii_lowercase();
    match code {
        RefusalCode::Encrypted => n.contains("encrypted"),
        RefusalCode::CannotShrinkType => n.contains("random") || n.contains("corrupt"),
        RefusalCode::TotalTooBig => n.contains("random"),
        RefusalCode::TooLongForLimit { .. } => preset != "smaller",
        RefusalCode::BelowQualityFloor => {
            n.contains("huge") || n.contains("48mp") || n.contains("big")
        }
        RefusalCode::UnsupportedInput { .. } => {
            n.ends_with(".rar") || n.contains("heic") || n.contains("avif")
        }
        RefusalCode::NeedsFfmpeg => true,
        // Only honest when this machine's FFmpeg really has no working H.264 encoder.
        RefusalCode::NoEncoder { .. } => !has_h264,
        _ => false,
    }
}

fn expected_failure(name: &str, input_bytes: u64) -> bool {
    let n = name.to_ascii_lowercase();
    input_bytes == 0 || n.contains("corrupt") || n.contains("truncated") || n.contains("encrypted")
}
