//! End-to-end engine jobs with the in-memory reader and sink: per-file and
//! per-message limits, naming, packaging, refusals and cancellation.

use cia_core::events::VecSink;
use cia_core::*;
use cia_engine::inspect::InputSpec;
use cia_engine::output::MemorySink;
use cia_engine::{CancelToken, Engine, EngineError, InputReader, PlanRequest};
use image::{ImageEncoder, RgbImage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct MemReader(Mutex<HashMap<String, Vec<u8>>>);
impl InputReader for MemReader {
    fn read(&self, source: &SourceRef) -> Result<Vec<u8>, EngineError> {
        let SourceRef::Handle { handle_id } = source else {
            return Err(EngineError::Unsupported("path".into()));
        };
        self.0
            .lock()
            .unwrap()
            .get(handle_id)
            .cloned()
            .ok_or_else(|| EngineError::Io("source_vanished".into()))
    }
    fn len(&self, source: &SourceRef) -> Result<u64, EngineError> {
        self.read(source).map(|b| b.len() as u64)
    }
}

fn photo(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut img = RgbImage::new(w, h);
    let mut s = seed.wrapping_mul(2_654_435_761) | 1;
    for (x, y, p) in img.enumerate_pixels_mut() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        let n = (s % 48) as i32 - 24;
        *p = image::Rgb([
            ((x * 255 / w) as i32 + n).clamp(0, 255) as u8,
            ((y * 255 / h) as i32 + n).clamp(0, 255) as u8,
            (((x + y) * 255 / (w + h)) as i32 - n).clamp(0, 255) as u8,
        ]);
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
    out
}

fn engine(files: &[(&str, Vec<u8>)]) -> (Engine, Arc<MemorySink>) {
    let map: HashMap<String, Vec<u8>> = files
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    let sink = Arc::new(MemorySink::default());
    let caps = Capabilities {
        host: "test".into(),
        opus_encode: false,
        ..Default::default()
    };
    (
        Engine::new(Arc::new(MemReader(Mutex::new(map))), sink.clone(), caps),
        sink,
    )
}

fn specs(files: &[(&str, Vec<u8>)], folder: Option<&str>) -> Vec<InputSpec> {
    files
        .iter()
        .map(|(k, _)| InputSpec {
            source: SourceRef::Handle {
                handle_id: k.to_string(),
            },
            rel_path: match folder {
                Some(f) => format!("{f}/{k}"),
                None => k.to_string(),
            },
            folder: folder.map(String::from),
        })
        .collect()
}

fn fit(preset: &str) -> Goal {
    let p = cia_core::presets::find(preset).unwrap();
    Goal::Fit {
        preset_id: p.id.clone(),
        limit: p.resolve().unwrap(),
    }
}

fn custom(bytes: u64, per_message: bool) -> Goal {
    Goal::Fit {
        preset_id: "custom".into(),
        limit: cia_core::presets::resolve_custom(bytes, per_message),
    }
}

#[test]
fn single_photo_fits_a_custom_limit_and_is_named_with_the_label() {
    let files = vec![("photo.jpg", photo(1200, 900, 1))];
    let (e, sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    assert_eq!(items[0].kind, Kind::Image);
    assert_eq!(
        (items[0].detail.width, items[0].detail.height),
        (Some(1200), Some(900))
    );
    let req = PlanRequest {
        items,
        goal: custom(60_000, false),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let plan = e.preview(&req, &CancelToken::new()).unwrap();
    assert!(
        matches!(plan.verdict, PlanVerdict::WillFit { .. }),
        "{:?}",
        plan.verdict
    );
    assert!(plan.items[0].prediction.exact);
    let events = VecSink::default();
    let (summary, log) = e.run(&req, &events, &CancelToken::new());
    assert_eq!(summary.verdict, JobVerdict::AllFit, "{}", summary.headline);
    let art = summary.outcomes[0].1.artifact().unwrap();
    assert_eq!(art.file_name, "photo (Custom 60 KB).jpg");
    assert!(art.bytes < 60_000);
    assert!(art.verification.passed());
    let written = sink.get(&art.file_name).expect("written through the sink");
    assert_eq!(
        written.len() as u64,
        art.bytes,
        "size read back equals the artifact size"
    );
    assert!(log.attempts.len() >= 2, "attempts logged");
    let ev = events.0.lock().unwrap();
    assert!(ev
        .iter()
        .any(|e| matches!(e, cia_core::events::EngineEvent::JobDone { .. })));
    assert!(ev
        .iter()
        .any(|e| matches!(e, cia_core::events::EngineEvent::Progress { .. })));
}

#[test]
fn collisions_never_overwrite() {
    let files = vec![("photo.jpg", photo(800, 600, 2))];
    let (e, sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: custom(40_000, false),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let (s1, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    let (s2, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    let n1 = s1.outcomes[0].1.artifact().unwrap().file_name.clone();
    let n2 = s2.outcomes[0].1.artifact().unwrap().file_name.clone();
    assert_eq!(n1, "photo (Custom 40 KB).jpg");
    assert_eq!(n2, "photo (Custom 40 KB 2).jpg");
    assert_eq!(sink.paths().len(), 2);
}

#[test]
fn already_fitting_jpeg_is_kept_and_nothing_is_written() {
    let files = vec![("small.jpg", photo(320, 240, 3))];
    let (e, sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: fit("discord-free"),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let plan = e.preview(&req, &CancelToken::new()).unwrap();
    assert!(plan.headline.contains("already fits"), "{}", plan.headline);
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    assert!(matches!(
        summary.outcomes[0].1,
        ItemOutcome::KeptOriginal { .. }
    ));
    assert!(sink.paths().is_empty());
}

#[test]
fn folder_to_email_is_all_or_nothing_and_zips_when_over_ten_files() {
    let files: Vec<(String, Vec<u8>)> = (0..12)
        .map(|i| (format!("IMG_{i}.jpg"), photo(640, 480, 10 + i)))
        .collect();
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let (e, sink) = engine(&refs);
    let items = e.inspect(&specs(&refs, Some("Grandkids")));
    let total: u64 = items.iter().map(|i| i.bytes).sum();
    let req = PlanRequest {
        items,
        goal: custom(total * 2 / 3, true),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let plan = e.preview(&req, &CancelToken::new()).unwrap();
    assert!(
        matches!(plan.packaging, PackagingPlan::Archive { .. }),
        "12 files on a per-message limit go into one zip: {:?}",
        plan.packaging
    );
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    assert_eq!(summary.verdict, JobVerdict::AllFit, "{}", summary.headline);
    let packaged = summary.packaged.expect("packaged artifact");
    assert_eq!(
        packaged.file_name,
        "Grandkids (Custom 1 MB).zip".replace("1 MB", &cia_core::format::mb_whole(total * 2 / 3))
    );
    assert!(packaged.bytes < total * 2 / 3);
    let zip = sink.get(&packaged.file_name).unwrap();
    let names: Vec<String> = summary
        .outcomes
        .iter()
        .map(|(_, o)| o.artifact().unwrap().file_name.clone())
        .collect();
    assert!(names.iter().all(|n| n.starts_with("IMG_")), "{names:?}");
    let report = cia_archive::verify(&zip, &names).expect("zip verifies with the planned names");
    assert_eq!(report.entry_count, 12);
    // Only the archive was written; no loose files.
    assert_eq!(sink.paths().len(), 1);
}

#[test]
fn per_message_total_too_big_refuses_before_writing() {
    let files = vec![
        ("a.jpg", photo(1600, 1200, 20)),
        ("b.jpg", photo(1600, 1200, 21)),
    ];
    let (e, sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: custom(20_000, true),
        packaging: Packaging::SeparateFiles,
        options: JobOptions::default(),
    };
    let plan = e.preview(&req, &CancelToken::new()).unwrap();
    match &plan.verdict {
        PlanVerdict::CannotFit { refusal } => {
            assert!(
                matches!(
                    refusal.code,
                    RefusalCode::TotalTooBig | RefusalCode::BelowQualityFloor
                ),
                "{:?}",
                refusal.code
            );
        }
        other => panic!("{other:?}"),
    }
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    assert_eq!(summary.verdict, JobVerdict::NoneFit);
    assert!(
        sink.paths().is_empty(),
        "nothing written: {:?}",
        sink.paths()
    );
}

#[test]
fn text_over_the_limit_becomes_a_zip_and_random_bytes_are_refused() {
    let text: Vec<u8> = "the quick brown fox jumps over the lazy dog "
        .repeat(20_000)
        .into_bytes();
    let mut random = vec![0u8; 400_000];
    let mut s = 12345u32;
    for b in random.iter_mut() {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *b = s as u8;
    }
    let files = vec![("notes.txt", text), ("blob.bin", random)];
    let (e, _sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    assert_eq!(items[0].kind, Kind::Text);
    assert_eq!(items[1].kind, Kind::Other);
    let req = PlanRequest {
        items,
        goal: custom(300_000, false),
        packaging: Packaging::SeparateFiles,
        options: JobOptions::default(),
    };
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    assert_eq!(summary.verdict, JobVerdict::SomeFit, "{}", summary.headline);
    let text_out = summary.outcomes[0].1.artifact().expect("text zipped");
    assert_eq!(text_out.file_name, "notes (Custom 300 KB).zip");
    assert!(text_out.bytes < 300_000);
    match &summary.outcomes[1].1 {
        ItemOutcome::Refused { refusal } => {
            assert!(matches!(refusal.code, RefusalCode::CannotShrinkType));
            assert!(
                refusal
                    .message
                    .contains("can't make this kind of file smaller"),
                "{}",
                refusal.message
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cancel_before_start_writes_nothing() {
    let files = vec![("photo.jpg", photo(1200, 900, 30))];
    let (e, sink) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: custom(50_000, false),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let token = CancelToken::new();
    token.cancel();
    let (summary, _) = e.run(&req, &VecSink::default(), &token);
    assert_eq!(summary.verdict, JobVerdict::Cancelled);
    assert_eq!(summary.headline, "Stopped. Nothing was saved.");
    assert!(sink.paths().is_empty());
}

#[test]
fn damaged_input_fails_with_the_plain_message() {
    let mut bad = photo(400, 300, 40);
    bad.truncate(500);
    let files = vec![("bad.jpg", bad)];
    let (e, _) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: custom(10_000, false),
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    match &summary.outcomes[0].1 {
        ItemOutcome::Failed { failure } => assert_eq!(
            failure.message,
            "This file is damaged or incomplete, so Smidge can't read it."
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn smaller_mode_single_file_keeps_source_format() {
    let files = vec![("photo.jpg", photo(1000, 750, 50))];
    let (e, _) = engine(&files);
    let items = e.inspect(&specs(&files, None));
    let req = PlanRequest {
        items,
        goal: Goal::Smaller {
            level: SmallerLevel::Smallest,
        },
        packaging: Packaging::Auto,
        options: JobOptions::default(),
    };
    let (summary, _) = e.run(&req, &VecSink::default(), &CancelToken::new());
    let art = summary.outcomes[0]
        .1
        .artifact()
        .expect("some outcome with an artifact");
    assert!(art.file_name.ends_with(".jpg"), "{}", art.file_name);
}
