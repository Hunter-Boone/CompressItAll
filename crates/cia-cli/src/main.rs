//! `cia`: the engine without the UI (DESIGN.md M1, 7.3).
//!   cia inspect <files...>
//!   cia plan --preset discord-free <files...>
//!   cia compress --preset discord-free [--out DIR] [--smaller] [--package zip] <files...>
//!   cia verify --preset discord-free <file>
//!   cia matrix --fixtures DIR --presets all|smoke|a,b,c --out DIR [--smoke]
//!   cia presets
use anyhow::{bail, Context, Result};
use cia_core::events::{EngineEvent, EventSink};
use cia_core::*;
use cia_engine::inspect::InputSpec;
use cia_engine::output::fs_sink::{FsReader, FsSink};
use cia_engine::{CancelToken, Engine, PlanRequest, VideoBackend};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "cia", version, about = "Smidge engine command line")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// App data directory (where FFmpeg lives). Defaults to the OS data dir + app.smidge.desktop.
    #[arg(long, global = true)]
    app_data: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Cmd {
    Inspect {
        files: Vec<PathBuf>,
    },
    Plan {
        #[arg(long, default_value = "discord-free")]
        preset: String,
        #[arg(long)]
        custom_mb: Option<f64>,
        #[arg(long)]
        smaller: bool,
        files: Vec<PathBuf>,
    },
    Compress {
        #[arg(long, default_value = "discord-free")]
        preset: String,
        #[arg(long)]
        custom_mb: Option<f64>,
        #[arg(long)]
        smaller: bool,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "auto")]
        package: String,
        #[arg(long)]
        log: Option<PathBuf>,
        files: Vec<PathBuf>,
    },
    Verify {
        #[arg(long, default_value = "discord-free")]
        preset: String,
        file: PathBuf,
    },
    Matrix {
        #[arg(long)]
        fixtures: PathBuf,
        #[arg(long, default_value = "smoke")]
        presets: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        smoke: bool,
        #[arg(long)]
        only: Option<String>,
    },
    Presets,
}

struct Printer {
    json: bool,
}
impl EventSink for Printer {
    fn emit(&self, e: EngineEvent) {
        match &e {
            EngineEvent::Progress {
                item_id,
                fraction,
                label,
                ..
            } => {
                if !self.json {
                    eprint!(
                        "\r  {label:<24} {:>3}%  {item_id}   ",
                        (fraction * 100.0) as u32
                    );
                }
            }
            EngineEvent::ItemDone {
                outcome, item_id, ..
            } if !self.json => {
                eprintln!("\r  {item_id}: {}", short_outcome(outcome));
            }
            _ => {}
        }
    }
}

fn short_outcome(o: &ItemOutcome) -> String {
    match o {
        ItemOutcome::Fitted { artifact } => format!(
            "fitted {} ({} bytes, {})",
            artifact.file_name, artifact.bytes, artifact.summary
        ),
        ItemOutcome::KeptOriginal { artifact } => {
            format!("kept original ({} bytes)", artifact.bytes)
        }
        ItemOutcome::Refused { refusal } => format!("refused: {}", refusal.message),
        ItemOutcome::Failed { failure } => {
            format!("failed [{}]: {}", failure.code, failure.message)
        }
        ItemOutcome::Cancelled => "cancelled".into(),
    }
}

fn app_data(cli: &Cli) -> PathBuf {
    cli.app_data.clone().unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("app.smidge.desktop")
    })
}

fn engine(cli: &Cli, job_id: &str) -> Engine {
    let caps = Capabilities {
        host: "cli".into(),
        can_copy_files: false,
        can_drag_out: false,
        can_reveal: true,
        can_choose_folder: true,
        heic_input: false,
        avif_input: false,
        opus_encode: true,
        mp3_encode: false,
        aac_encode: false,
        video: false,
        ..Default::default()
    };
    let mut e = Engine::new(Arc::new(FsReader), Arc::new(FsSink::new(job_id)), caps);
    if let Some(ff) = cia_engine::video_ffmpeg::FfmpegBackend::locate(&app_data(cli)) {
        let caps = ff.capabilities();
        e = e.with_video(Arc::new(ff));
        if let Some(c) = caps {
            e.caps.mp3_encode = c.has_libmp3lame;
            e.caps.aac_encode = c.has_aac;
            e.caps.heic_input = true;
            e.caps.avif_input = true;
        }
    }
    e
}

fn specs(files: &[PathBuf]) -> Result<Vec<InputSpec>> {
    let mut out = Vec::new();
    for f in files {
        if f.is_dir() {
            let folder = f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            for entry in walk(f)? {
                let rel = format!(
                    "{}/{}",
                    folder,
                    entry
                        .strip_prefix(f)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                out.push(InputSpec {
                    source: SourceRef::Path {
                        path: entry.to_string_lossy().to_string(),
                    },
                    rel_path: rel,
                    folder: Some(folder.clone()),
                });
            }
        } else {
            if !f.exists() {
                bail!("{} does not exist", f.display());
            }
            out.push(InputSpec {
                source: SourceRef::Path {
                    path: f.to_string_lossy().to_string(),
                },
                rel_path: f
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                folder: None,
            });
        }
    }
    Ok(out)
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if p.is_dir() {
            v.extend(walk(&p)?);
        } else {
            v.push(p);
        }
    }
    v.sort();
    Ok(v)
}

fn goal(preset: &str, custom_mb: Option<f64>, smaller: bool) -> Result<Goal> {
    if smaller || preset == "smaller" {
        return Ok(Goal::Smaller {
            level: SmallerLevel::KeepQuality,
        });
    }
    if let Some(mb) = custom_mb {
        return Ok(Goal::Fit {
            preset_id: "custom".into(),
            limit: cia_core::presets::resolve_custom((mb * 1_000_000.0) as u64, false),
        });
    }
    let p = cia_core::presets::find(preset).with_context(|| format!("unknown preset {preset}"))?;
    Ok(Goal::Fit {
        preset_id: p.id.clone(),
        limit: p.resolve()?,
    })
}

fn packaging(s: &str) -> Packaging {
    match s {
        "separate" => Packaging::SeparateFiles,
        "zip" => Packaging::Zip,
        "7z" => Packaging::SevenZip,
        "tar.zst" => Packaging::TarZst,
        "tar.xz" => Packaging::TarXz,
        _ => Packaging::Auto,
    }
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Presets => {
            for p in cia_core::presets::all() {
                let limit = p
                    .limit
                    .as_ref()
                    .map(|l| format!("{} ({} bytes, {:?})", l.stated_as, l.bytes, l.scope))
                    .unwrap_or_else(|| "none".into());
                println!(
                    "{:<22} {:<28} {}",
                    p.id,
                    format!(
                        "{}{}",
                        p.tile_label,
                        p.tier_label
                            .as_ref()
                            .map(|t| format!(" / {t}"))
                            .unwrap_or_default()
                    ),
                    limit
                );
            }
        }
        Cmd::Inspect { files } => {
            let e = engine(&cli, "inspect");
            let items = e.inspect(&specs(files)?);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&items)?);
            } else {
                for i in &items {
                    println!(
                        "{:<40} {:>12}  {:?} {}  {}",
                        i.rel_path,
                        i.bytes,
                        i.kind,
                        i.detail.format,
                        cia_core::copy::describe_input(i)
                    );
                }
            }
        }
        Cmd::Plan {
            preset,
            custom_mb,
            smaller,
            files,
        } => {
            let e = engine(&cli, "plan");
            let items = e.inspect(&specs(files)?);
            let req = PlanRequest {
                items,
                goal: goal(preset, *custom_mb, *smaller)?,
                packaging: Packaging::Auto,
                options: JobOptions::default(),
            };
            let plan = e.preview(&req, &CancelToken::new())?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!("{}", plan.headline);
                for p in &plan.items {
                    println!(
                        "  {:<28} budget {:?}  predicted {}  {}",
                        p.item_id,
                        p.budget_bytes,
                        p.prediction.predicted_bytes,
                        p.prediction.summary
                    );
                }
                if let PlanVerdict::CannotFit { refusal } = &plan.verdict {
                    println!("  refusal: {:?}", refusal.code);
                }
            }
        }
        Cmd::Compress {
            preset,
            custom_mb,
            smaller,
            out,
            package,
            log,
            files,
        } => {
            let job_id = cia_core::new_id();
            let e = engine(&cli, &job_id);
            let items = e.inspect(&specs(files)?);
            let mut options = JobOptions::default();
            if let Some(o) = out {
                std::fs::create_dir_all(o)?;
                options.output_dir = Some(OutputDir::Folder {
                    path: o.to_string_lossy().to_string(),
                });
            }
            let req = PlanRequest {
                items,
                goal: goal(preset, *custom_mb, *smaller)?,
                packaging: packaging(package),
                options,
            };
            let (summary, jlog) = e.run(&req, &Printer { json: cli.json }, &CancelToken::new());
            if let Some(l) = log {
                std::fs::write(l, jlog.to_json())?;
            }
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&summary)?);
            } else {
                println!("\n{}", summary.headline);
                println!(
                    "{} → {} in {} ms",
                    cia_core::format::mb(summary.input_bytes),
                    cia_core::format::mb(summary.total_bytes),
                    summary.elapsed_ms
                );
                if let Some(p) = &summary.packaged {
                    println!("packaged: {} ({} bytes)", p.file_name, p.bytes);
                }
            }
            if summary.verdict == JobVerdict::NoneFit {
                std::process::exit(1);
            }
        }
        Cmd::Verify { preset, file } => {
            let g = goal(preset, None, false)?;
            let hard = g.limit().map(|l| l.hard_bytes);
            let bytes = std::fs::read(file)?;
            let size_ok = hard.is_none_or(|h| (bytes.len() as u64) < h);
            let head = &bytes[..bytes.len().min(65536)];
            let det = cia_engine::detect::detect(head, &file.to_string_lossy());
            let decodes = match det.kind {
                Kind::Image | Kind::AnimatedImage => image::load_from_memory(&bytes).is_ok(),
                Kind::Archive | Kind::OfficeDoc => cia_archive::open(&bytes)
                    .map(|mut a| a.extract_all().is_ok())
                    .unwrap_or(false),
                Kind::Pdf => bytes.starts_with(b"%PDF"),
                _ => !bytes.is_empty(),
            };
            println!(
                "{} {} bytes {:?} {}  size_ok={size_ok} decodes={decodes}",
                file.display(),
                bytes.len(),
                det.kind,
                det.format
            );
            if !size_ok || !decodes {
                std::process::exit(2);
            }
        }
        Cmd::Matrix {
            fixtures,
            presets,
            out,
            smoke,
            only,
        } => {
            matrix::run(&cli, fixtures, presets, out, *smoke, only.as_deref())?;
        }
    }
    Ok(())
}

mod matrix;
