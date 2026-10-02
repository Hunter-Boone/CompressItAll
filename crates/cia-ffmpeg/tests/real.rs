//! Tests that run a real FFmpeg (DESIGN.md 7.1, cia-ffmpeg row). They skip
//! with a message when no FFmpeg or no fixtures are available, so the suite
//! stays green on a bare machine but says so.

mod support;

use cia_ffmpeg::command::Encoder;
use cia_ffmpeg::runner::{run_ffmpeg, RunOptions};
use cia_ffmpeg::{
    encoders, Cancel, CompressRequest, ExpectedOutput, FfmpegError, Session, VideoOutcome,
};
use cia_video_plan::{AudioCodec, Budget, Container, PlanOptions, Target, VideoCodec};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn budget(bytes: u64) -> Budget {
    Budget {
        raw_budget_bytes: bytes,
        hard_bytes: bytes,
        safety_bytes: 0,
    }
}

fn h264_target() -> Target {
    Target::new(Container::Mp4, VideoCodec::H264, AudioCodec::Aac)
}

fn mp4_format() -> cia_core::presets::VideoFormat {
    cia_core::presets::VideoFormat {
        container: "mp4".into(),
        video: "h264".into(),
        audio: "aac".into(),
    }
}

/// Encoders available on this FFmpeg for an H.264 MP4 target (libx264 first).
fn chain(session: &Session) -> Vec<Encoder> {
    let report = encoders::run_detection(&session.installed, &mut |_| {}, &Cancel::new())
        .expect("encoder detection");
    let mut chain = report.chain(true, true);
    if chain.is_empty() {
        panic!("this FFmpeg has no usable H.264 or VP9 encoder: {report:?}");
    }
    chain.retain(|e| *e != Encoder::LibvpxVp9 || !report.works("libx264"));
    chain
}

fn compress(
    session: &Session,
    input: &Path,
    bytes: u64,
    options: &PlanOptions,
    allowed: &[cia_core::presets::VideoFormat],
    sink: &Path,
    stem: &str,
) -> cia_ffmpeg::VideoResult {
    let probe = session.probe(input).expect("probe");
    let source_bytes = std::fs::metadata(input).unwrap().len();
    let chain = chain(session);
    let budget = budget(bytes);
    let target = h264_target();
    let req = CompressRequest {
        item_id: "item".into(),
        input,
        source_bytes,
        probe: &probe,
        budget: &budget,
        target: &target,
        options,
        encoders: &chain,
        allowed_formats: allowed,
        sink_dir: sink,
        output_stem: stem,
        max_size_attempts: 4,
    };
    let mut fractions = Vec::new();
    let result = session.compress_video(&req, &mut |f, _eta| fractions.push(f), &Cancel::new());
    if matches!(result.outcome, VideoOutcome::Fitted { .. }) {
        assert!(!fractions.is_empty(), "no progress was reported");
        assert!(
            fractions.windows(2).all(|w| w[1] >= w[0] - 1e-9) || result.attempts.len() > 1,
            "progress went backwards within one attempt: {fractions:?}"
        );
    }
    result
}

fn fitted(result: &cia_ffmpeg::VideoResult) -> (PathBuf, u64) {
    match &result.outcome {
        VideoOutcome::Fitted {
            path,
            bytes,
            verification,
            ..
        } => {
            assert!(verification.passed(), "verification: {verification:?}");
            (path.clone(), *bytes)
        }
        other => panic!(
            "expected Fitted, got {other:?}; attempts {:?}",
            result.attempts
        ),
    }
}

/// A short copy of a fixture (stream copy, `secs` seconds) to keep tests quick.
fn short_copy(session: &Session, input: &Path, out: &Path, secs: u32, extra: &[&str]) {
    let mut args: Vec<OsString> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
        "-i".into(),
        input.into(),
        "-t".into(),
        secs.to_string().into(),
        "-c".into(),
        "copy".into(),
    ];
    args.extend(extra.iter().map(OsString::from));
    args.push(out.into());
    run_ffmpeg(
        session.ffmpeg(),
        &args,
        &RunOptions::default(),
        &mut |_, _| {},
        &Cancel::new(),
    )
    .expect("stream copy");
}

#[test]
fn probe_720p_fixture() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let p = session.probe(&input).unwrap();
    assert_eq!(p.duration_ms, 10_000);
    assert_eq!((p.display_w, p.display_h), (1280, 720));
    assert_eq!(p.video_codec, "h264");
    assert_eq!(p.container, "mp4");
    assert_eq!(p.avg_fps, 30.0);
    assert!(!p.is_vfr);
    assert!(!p.is_hdr);
    assert_eq!(p.rotation_degrees, 0);
    assert_eq!(p.audio.len(), 1);
    assert_eq!(p.audio[0].codec, "aac");
}

#[test]
fn probe_two_audio_tracks_with_titles() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_two_audio_tracks.mkv"), "no fixtures");
    let p = Session::new(ff).probe(&input).unwrap();
    assert_eq!(p.container, "mkv");
    assert_eq!(p.audio.len(), 2);
    assert_eq!(p.audio[0].title.as_deref(), Some("Game"));
    assert_eq!(p.audio[1].title.as_deref(), Some("Mic"));
    assert_eq!(p.audio[1].index, 1);
    assert_eq!(p.audio[0].codec, "vorbis");
}

#[test]
fn probe_vfr_fixture_is_vfr() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_vfr.mp4"), "no fixtures");
    let p = Session::new(ff).probe(&input).unwrap();
    assert!(p.is_vfr, "{p:?}");
}

#[test]
fn probe_noaudio_fixture() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_noaudio_10s.mp4"), "no fixtures");
    let p = Session::new(ff).probe(&input).unwrap();
    assert!(p.audio.is_empty());
}

#[test]
fn truncated_input_fails_with_damaged_message() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_truncated.mp4"), "no fixtures");
    let err = Session::new(ff).probe(&input).unwrap_err();
    assert!(matches!(err, FfmpegError::DamagedInput { .. }), "{err:?}");
    assert_eq!(err.failure_code(), "damaged_video");
    assert_eq!(
        err.user_message(),
        "This video file is damaged or incomplete, so Smidge can't read it."
    );
}

#[test]
fn compress_720p_to_custom_budget() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    const BUDGET: u64 = 1_500_000;
    let result = compress(
        &session,
        &input,
        BUDGET,
        &PlanOptions::default(),
        &[],
        sink.path(),
        "clip [Discord]",
    );
    let (path, bytes) = fitted(&result);
    assert!(bytes <= BUDGET, "{bytes} > {BUDGET}");
    assert!(path.is_file());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), bytes);
    let out = session.probe(&path).unwrap();
    assert!(
        out.duration_ms.abs_diff(10_000) <= 100,
        "{}",
        out.duration_ms
    );
    assert_eq!(out.video_codec, "h264");
    assert_eq!(out.container, "mp4");
    assert!(!out.is_vfr);
    let plan = result.plan.as_ref().unwrap();
    eprintln!(
        "BUDGET ACCURACY: budget {BUDGET}, predicted {}, actual {bytes} ({:.1} percent of budget), rung {}x{}@{} video {} bps audio {} bps, attempts {}",
        plan.predicted_bytes,
        bytes as f64 / BUDGET as f64 * 100.0,
        plan.width,
        plan.height,
        plan.fps,
        plan.video_bps,
        plan.audio_bps,
        result.attempts.len()
    );
    assert!(matches!(
        result.attempts.last().unwrap().verdict,
        cia_core::AttemptVerdict::Fits
    ));
    // no partial left behind
    let leftovers: Vec<_> = std::fs::read_dir(sink.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn portrait_stays_portrait() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(
        support::fixture("v_portrait_1080x1920_10s.mp4"),
        "no fixtures"
    );
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    let result = compress(
        &session,
        &input,
        1_500_000,
        &PlanOptions::default(),
        &[],
        sink.path(),
        "portrait",
    );
    let (path, _) = fitted(&result);
    let out = session.probe(&path).unwrap();
    assert!(
        out.display_h > out.display_w,
        "{}x{}",
        out.display_w,
        out.display_h
    );
    assert_eq!(out.rotation_degrees, 0);
}

#[test]
fn rotated_input_comes_out_upright() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let work = tempfile::tempdir().unwrap();
    // The synthetic fixture generator could not tag rotation with FFmpeg 5.1,
    // so build a rotation-tagged clip here: landscape pixels, display matrix 90.
    let rotated = work.path().join("rotated.mp4");
    short_copy(
        &session,
        &input,
        &rotated,
        3,
        &["-metadata:s:v:0", "rotate=90"],
    );
    let p = session.probe(&rotated).unwrap();
    assert_eq!((p.coded_w, p.coded_h), (1280, 720));
    assert!(p.is_portrait(), "probe did not see the rotation: {p:?}");
    assert!(
        p.rotation_degrees == 90 || p.rotation_degrees == 270,
        "{}",
        p.rotation_degrees
    );

    let result = compress(
        &session,
        &rotated,
        1_200_000,
        &PlanOptions::default(),
        &[],
        work.path(),
        "upright",
    );
    let (path, _) = fitted(&result);
    let out = session.probe(&path).unwrap();
    assert!(
        out.display_h > out.display_w,
        "{}x{}",
        out.display_w,
        out.display_h
    );
    assert_eq!(
        out.rotation_degrees, 0,
        "output must carry no rotation side data"
    );
    let plan = result.plan.unwrap();
    assert_eq!((out.coded_w, out.coded_h), (plan.width, plan.height));
}

#[test]
fn two_audio_tracks_mix_to_one_stream() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_two_audio_tracks.mkv"), "no fixtures");
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    let result = compress(
        &session,
        &input,
        2_000_000,
        &PlanOptions::default(),
        &[mp4_format()],
        sink.path(),
        "mixed",
    );
    let (path, _) = fitted(&result);
    let out = session.probe(&path).unwrap();
    assert_eq!(out.audio.len(), 1, "{:?}", out.audio);
    assert_eq!(out.audio[0].codec, "aac");
    assert_eq!(out.container, "mp4");
    // two tracks never keep-original or remux (DECISIONS.md)
    assert!(result.attempts.iter().all(|a| a.encoder != "remux"));
}

#[test]
fn vfr_input_comes_out_cfr() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_vfr.mp4"), "no fixtures");
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    let result = compress(
        &session,
        &input,
        1_500_000,
        &PlanOptions::default(),
        &[],
        sink.path(),
        "cfr",
    );
    let (path, _) = fitted(&result);
    let out = session.probe(&path).unwrap();
    assert!(!out.is_vfr, "{out:?}");
    assert_eq!(out.avg_fps, out.max_fps);
    assert!(
        out.duration_ms.abs_diff(10_000) <= 100,
        "{}",
        out.duration_ms
    );
}

#[test]
fn source_that_fits_is_kept_or_remuxed() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let work = tempfile::tempdir().unwrap();
    let source_bytes = std::fs::metadata(&input).unwrap().len();

    let result = compress(
        &session,
        &input,
        source_bytes + 1_000_000,
        &PlanOptions::default(),
        &[mp4_format()],
        work.path(),
        "kept",
    );
    assert_eq!(result.outcome, VideoOutcome::KeptOriginal);
    assert!(result.attempts.is_empty());

    // Same streams in MKV: remux, no re-encode.
    let mkv = work.path().join("same streams.mkv");
    short_copy(&session, &input, &mkv, 10, &[]);
    let result = compress(
        &session,
        &mkv,
        source_bytes + 1_000_000,
        &PlanOptions::default(),
        &[mp4_format()],
        work.path(),
        "remuxed",
    );
    match &result.outcome {
        VideoOutcome::Fitted {
            path,
            remuxed,
            verification,
            ..
        } => {
            assert!(*remuxed, "{:?}", result.attempts);
            assert!(verification.passed(), "{verification:?}");
            assert_eq!(path.extension().unwrap(), "mp4");
            assert_eq!(session.probe(path).unwrap().container, "mp4");
        }
        other => panic!("{other:?} {:?}", result.attempts),
    }
    assert_eq!(result.attempts.len(), 1);
    assert_eq!(result.attempts[0].encoder, "remux");
}

#[test]
fn trim_shortens_the_output() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    let options = PlanOptions {
        trim: Some((2_000, 5_000)),
        ..PlanOptions::default()
    };
    let result = compress(
        &session,
        &input,
        1_000_000,
        &options,
        &[],
        sink.path(),
        "trimmed",
    );
    let (path, _) = fitted(&result);
    let out = session.probe(&path).unwrap();
    assert!(
        out.duration_ms.abs_diff(3_000) <= 100,
        "{}",
        out.duration_ms
    );
}

#[test]
fn remove_sound_drops_audio() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let work = tempfile::tempdir().unwrap();
    let short = work.path().join("short.mp4");
    short_copy(&session, &input, &short, 2, &[]);
    let options = PlanOptions {
        audio: cia_core::AudioTrackChoice::Remove,
        ..PlanOptions::default()
    };
    let result = compress(
        &session,
        &short,
        600_000,
        &options,
        &[],
        work.path(),
        "silent",
    );
    let (path, _) = fitted(&result);
    assert!(session.probe(&path).unwrap().audio.is_empty());
}

#[test]
fn cancel_removes_the_partial_file() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let sink = tempfile::tempdir().unwrap();
    let probe = session.probe(&input).unwrap();
    let chain = chain(&session);
    let budget = budget(1_500_000);
    let target = h264_target();
    let options = PlanOptions::default();
    let req = CompressRequest {
        item_id: "cancel".into(),
        input: &input,
        source_bytes: std::fs::metadata(&input).unwrap().len(),
        probe: &probe,
        budget: &budget,
        target: &target,
        options: &options,
        encoders: &chain,
        allowed_formats: &[],
        sink_dir: sink.path(),
        output_stem: "cancelled",
        max_size_attempts: 4,
    };
    let cancel = Cancel::new();
    let c2 = cancel.clone();
    let mut seen = 0;
    let result = session.compress_video(
        &req,
        &mut |_, _| {
            seen += 1;
            if seen == 2 {
                c2.cancel();
            }
        },
        &cancel,
    );
    assert_eq!(
        result.outcome,
        VideoOutcome::Cancelled,
        "{:?}",
        result.attempts
    );
    let files: Vec<_> = std::fs::read_dir(sink.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(files.is_empty(), "left behind: {files:?}");
    assert!(matches!(
        result.attempts.last().map(|a| &a.verdict),
        Some(cia_core::AttemptVerdict::Cancelled)
    ));
}

#[test]
fn watchdog_kills_a_stalled_process() {
    // A process that never prints progress stands in for a hung encoder.
    let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
        ("ping", vec!["-n", "60", "127.0.0.1"])
    } else {
        ("sleep", vec!["60"])
    };
    let program = if cfg!(windows) {
        PathBuf::from(program)
    } else {
        ["/bin/sleep", "/usr/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .expect("sleep binary")
    };
    let work = tempfile::tempdir().unwrap();
    let partial = work.path().join("stalled.partial.mp4");
    std::fs::write(&partial, b"half a file").unwrap();
    let opts = RunOptions {
        stall_timeout: Duration::from_millis(400),
        partial_files: vec![partial.clone()],
        ..RunOptions::default()
    };
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let started = std::time::Instant::now();
    let err = run_ffmpeg(&program, &args, &opts, &mut |_, _| {}, &Cancel::new()).unwrap_err();
    assert!(matches!(err, FfmpegError::Stalled), "{err:?}");
    assert_eq!(err.failure_code(), "encoder_stalled");
    assert_eq!(err.user_message(), "The video encoder stopped responding.");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
    assert!(!partial.exists(), "partial file was not removed");
}

#[test]
fn cancel_token_kills_a_running_process() {
    let program = if cfg!(windows) {
        PathBuf::from("ping")
    } else {
        ["/bin/sleep", "/usr/bin/sleep"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
            .expect("sleep binary")
    };
    let args: Vec<OsString> = if cfg!(windows) {
        ["-n", "60", "127.0.0.1"]
            .iter()
            .map(OsString::from)
            .collect()
    } else {
        vec!["60".into()]
    };
    let cancel = Cancel::new();
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c2.cancel();
    });
    let started = std::time::Instant::now();
    let err = run_ffmpeg(
        &program,
        &args,
        &RunOptions::default(),
        &mut |_, _| {},
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(err, FfmpegError::Cancelled), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn paths_with_spaces_emoji_and_cjk_work_end_to_end() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let work = tempfile::tempdir().unwrap();
    let in_dir = work.path().join("Grandkids 🎂 誕生日");
    std::fs::create_dir_all(&in_dir).unwrap();
    let weird = in_dir.join("Grandkids 🎂 誕生日 (final) ;&|.mp4");
    short_copy(&session, &input, &weird, 2, &[]);
    assert!(weird.is_file());
    let p = session.probe(&weird).unwrap();
    assert_eq!(p.display_w, 1280);

    let frame = session.extract_frame(&weird, 1_000).unwrap();
    assert_eq!(&frame[..2], &[0xFF, 0xD8]);

    let result = compress(
        &session,
        &weird,
        700_000,
        &PlanOptions::default(),
        &[],
        &in_dir,
        "Grandkids 🎂 誕生日 (final) [Discord] ;&|",
    );
    let (path, _) = fitted(&result);
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "Grandkids 🎂 誕生日 (final) [Discord] ;&|.mp4"
    );
    assert!(path.is_file());
}

#[test]
fn extract_frame_returns_a_jpeg() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let session = Session::new(ff);
    let jpeg = session.extract_frame(&input, 4_500).unwrap();
    let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
    assert_eq!(img.width(), 1280);
    assert_eq!(img.height(), 720);
    // past the end: no frame
    let err = session.extract_frame(&input, 60_000).unwrap_err();
    assert!(
        matches!(err, FfmpegError::BadOutput(_) | FfmpegError::Exit { .. }),
        "{err:?}"
    );
}

#[test]
fn decode_image_through_ffmpeg() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let session = Session::new(ff);
    let work = tempfile::tempdir().unwrap();
    let png = work.path().join("red 🎂.png");
    let mut img = image::RgbaImage::new(20, 10);
    for p in img.pixels_mut() {
        *p = image::Rgba([255, 0, 0, 128]);
    }
    img.save(&png).unwrap();
    let decoded = session.decode_image(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (20, 10));
    assert_eq!(decoded.get_pixel(3, 3).0, [255, 0, 0, 128]);
    let err = session
        .decode_image(&work.path().join("missing.heic"))
        .unwrap_err();
    assert!(matches!(err, FfmpegError::Exit { .. }), "{err:?}");
}

#[test]
fn encoder_detection_and_cache() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let work = tempfile::tempdir().unwrap();
    let cache = work.path().join("encoders.json");
    let mut tested = Vec::new();
    let report = encoders::detect(
        &ff,
        Some(&cache),
        &mut |n| tested.push(n.to_string()),
        &Cancel::new(),
    )
    .unwrap();
    assert!(cache.is_file());
    assert!(report.works("aac"), "{report:?}");
    assert!(!tested.is_empty());
    for name in ["libx264", "libvpx-vp9", "h264_nvenc", "h264_vaapi"] {
        let listed = report.listed.iter().any(|l| l == name);
        let decided = report.works(name)
            || report.failed.contains_key(name)
            || report.skipped.iter().any(|s| s == name);
        assert!(decided, "{name} listed={listed} but undecided: {report:?}");
    }
    let caps = report.capabilities(&ff);
    assert!(caps.has_aac);
    assert_eq!(caps.has_libx264, report.works("libx264"));
    // second call comes from the cache without running anything
    let mut tested_again = Vec::new();
    let again = encoders::detect(
        &ff,
        Some(&cache),
        &mut |n| tested_again.push(n.to_string()),
        &Cancel::new(),
    )
    .unwrap();
    assert_eq!(again, report);
    assert!(tested_again.is_empty());
}

#[test]
fn verify_output_catches_a_wrong_file() {
    let ff = need!(support::ffmpeg(), "no ffmpeg");
    let input = need!(support::fixture("v_720p_30fps_10s.mp4"), "no fixtures");
    let truncated = need!(support::fixture("v_truncated.mp4"), "no fixtures");
    let session = Session::new(ff);
    let expected = ExpectedOutput {
        container: Container::Mp4,
        video_codec: VideoCodec::H264,
        duration_ms: 10_000,
        fps: 30.0,
        width: 1280,
        height: 720,
        has_audio: true,
        hard_bytes: 100_000_000,
    };
    let ok = session.verify_output(&input, &expected);
    assert!(ok.passed(), "{ok:?}");

    let bad = session.verify_output(&truncated, &expected);
    assert!(!bad.passed());
    assert!(!bad.decodes);

    let wrong = ExpectedOutput {
        duration_ms: 12_000,
        width: 720,
        height: 1280,
        has_audio: false,
        hard_bytes: 1_000,
        ..expected
    };
    let r = session.verify_output(&input, &wrong);
    assert!(!r.size_ok);
    assert!(r.failures.iter().any(|f| f.contains("duration")), "{r:?}");
    assert!(r.failures.iter().any(|f| f.contains("portrait")), "{r:?}");
    assert!(
        r.failures.iter().any(|f| f.contains("audio present")),
        "{r:?}"
    );
}
