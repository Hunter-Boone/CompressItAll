//! Round trips, planner tables and the verification predicate (DESIGN.md 7.1).

use cia_audio::plan::{
    plan_fit, plan_smaller, predict_bytes, retry_scale, AudioFormat, AudioPlan, HostAudioCaps,
    SmallerDecision, MAX_ATTEMPTS,
};
use cia_audio::{
    decode, downmix_mono, encode_flac, encode_wav, probe, resample, verify, AudioInfo, Decoded,
};
use cia_core::{RefusalCode, SmallerLevel};

const MP3: &[u8] = include_bytes!("fixtures/a_10s.mp3");
const OGG_VORBIS: &[u8] = include_bytes!("fixtures/a_10s.ogg");

/// Stereo sine (440 Hz left, 660 Hz right) plus deterministic noise at -40 dB,
/// quantised to 16 bits so lossless round trips can be compared bit-exactly.
fn synth(seconds: f32, rate: u32, channels: u16) -> Decoded {
    let frames = (seconds * rate as f32) as usize;
    let mut seed: u32 = 0x9E37_79B9;
    let mut samples = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        for c in 0..channels {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let freq = 440.0 * (1.0 + 0.5 * c as f32);
            let s = 0.5 * (2.0 * std::f32::consts::PI * freq * t).sin() + 0.01 * noise;
            let q = (s * 32767.0).round().clamp(-32768.0, 32767.0);
            samples.push(q / 32768.0);
        }
    }
    Decoded::new(rate, channels, samples, "pcm")
}

#[test]
fn wav_to_flac_round_trip_is_bit_exact_and_smaller() {
    let src = synth(10.0, 44_100, 2);
    let wav = encode_wav(&src);
    assert_eq!(wav.len(), 44 + src.samples.len() * 2);

    let from_wav = decode(&wav, None).unwrap();
    assert_eq!(from_wav.channels, 2);
    assert_eq!(from_wav.sample_rate, 44_100);
    assert_eq!(from_wav.duration_ms, 10_000);
    assert_eq!(
        from_wav.samples, src.samples,
        "WAV decode must be bit-exact"
    );

    let flac = encode_flac(&from_wav, 8).unwrap();
    assert!(
        flac.len() < wav.len() * 80 / 100,
        "FLAC {} should be well under WAV {}",
        flac.len(),
        wav.len()
    );
    eprintln!(
        "flac {} bytes vs wav {} bytes, ratio {:.3}",
        flac.len(),
        wav.len(),
        flac.len() as f64 / wav.len() as f64
    );
    let from_flac = decode(&flac, None).unwrap();
    assert_eq!(from_flac.source_codec, "flac");
    assert_eq!(
        from_flac.samples, src.samples,
        "FLAC round trip must be bit-exact"
    );

    let info = probe(&flac).unwrap();
    assert_eq!(info.codec, "flac");
    assert!(info.lossless);
    assert!(!info.needs_ffmpeg);
    assert_eq!(info.channels, 2);
    assert_eq!(info.duration_ms, 10_000);

    // A lower level still round trips and is not smaller than level 8.
    let flac0 = encode_flac(&from_wav, 0).unwrap();
    assert_eq!(decode(&flac0, None).unwrap().samples, src.samples);
    assert!(flac0.len() >= flac.len());
}

#[test]
fn mp3_and_vorbis_fixtures_decode_to_the_right_duration_and_channels() {
    let info = probe(MP3).unwrap();
    assert_eq!(info.codec, "mp3");
    assert_eq!(info.channels, 1);
    assert_eq!(info.sample_rate, 44_100);
    assert!(
        (9_900..=10_200).contains(&info.duration_ms),
        "mp3 duration {}",
        info.duration_ms
    );
    assert!(!info.lossless);
    let br = info.bitrate_bps.unwrap();
    assert!((300_000..=340_000).contains(&br), "mp3 bitrate {br}");

    let d = decode(MP3, None).unwrap();
    assert_eq!(d.channels, 1);
    assert_eq!(d.sample_rate, 44_100);
    assert!(
        (9_900..=10_200).contains(&d.duration_ms),
        "{}",
        d.duration_ms
    );
    assert_eq!(d.source_codec, "mp3");

    let info = probe(OGG_VORBIS).unwrap();
    assert_eq!(info.codec, "vorbis");
    assert_eq!(info.channels, 1);
    assert!(
        (9_950..=10_050).contains(&info.duration_ms),
        "vorbis duration {}",
        info.duration_ms
    );
    let d = decode(OGG_VORBIS, None).unwrap();
    assert_eq!(d.channels, 1);
    assert_eq!(d.sample_rate, 44_100);
    assert!(
        (9_950..=10_050).contains(&d.duration_ms),
        "{}",
        d.duration_ms
    );
    assert_eq!(d.source_codec, "vorbis");

    // max_seconds truncates.
    let head = decode(MP3, Some(2.0)).unwrap();
    assert_eq!(head.duration_ms, 2_000);
}

#[test]
fn probe_flags_ffmpeg_only_containers() {
    let mut m4a = vec![0, 0, 0, 0x20];
    m4a.extend_from_slice(b"ftypM4A ");
    m4a.extend_from_slice(&[0u8; 64]);
    let info = probe(&m4a).unwrap();
    assert_eq!(info.codec, "aac");
    assert!(info.needs_ffmpeg);
    assert!(matches!(
        decode(&m4a, None),
        Err(cia_audio::AudioError::NeedsFfmpeg(c)) if c == "aac"
    ));

    let mut alac = m4a.clone();
    alac.extend_from_slice(b"alac");
    let info = probe(&alac).unwrap();
    assert_eq!(info.codec, "alac");
    assert!(info.lossless && info.needs_ffmpeg);

    let mut wma = vec![
        0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE,
        0x6C,
    ];
    wma.extend_from_slice(&[0u8; 64]);
    assert_eq!(probe(&wma).unwrap().codec, "wma");

    assert!(probe(b"this is not audio at all, just text padding padding").is_err());
    assert!(matches!(probe(&[]), Err(cia_audio::AudioError::Empty)));
}

#[test]
fn resample_44k_to_48k_keeps_duration_and_signal() {
    let src = synth(10.0, 44_100, 2);
    let out = resample(&src, 48_000);
    assert_eq!(out.sample_rate, 48_000);
    assert_eq!(out.channels, 2);
    assert_eq!(out.frames(), 480_000);
    assert_eq!(out.duration_ms, 10_000);
    // RMS is preserved (within a few percent) by a proper resampler.
    let rms = |d: &Decoded| {
        (d.samples.iter().map(|s| (s * s) as f64).sum::<f64>() / d.samples.len() as f64).sqrt()
    };
    let (a, b) = (rms(&src), rms(&out));
    assert!((a - b).abs() / a < 0.05, "rms {a} vs {b}");
    // Round trip back down lands within 1 frame.
    let back = resample(&out, 44_100);
    assert_eq!(back.frames(), src.frames());
    // Identity is a clone.
    assert_eq!(resample(&src, 44_100), src);
}

#[test]
fn downmix_mono_averages_channels() {
    let src = synth(1.0, 48_000, 2);
    let mono = downmix_mono(&src);
    assert_eq!(mono.channels, 1);
    assert_eq!(mono.frames(), src.frames());
    assert_eq!(mono.duration_ms, 1_000);
    for (i, s) in mono.samples.iter().enumerate().take(100) {
        let expect = (src.samples[2 * i] + src.samples[2 * i + 1]) / 2.0;
        assert!((s - expect).abs() < 1e-6);
    }
    assert_eq!(downmix_mono(&mono), mono);
    let stereo = cia_audio::with_channels(&mono, 2);
    assert_eq!(stereo.channels, 2);
    assert_eq!(stereo.samples[0], stereo.samples[1]);
}

const ALL: [&str; 5] = ["mp3", "m4a_aac", "ogg_opus", "flac", "wav"];

#[test]
fn plan_fit_picks_the_highest_rung_that_fits() {
    let dur = 60_000;
    let caps = HostAudioCaps::all();
    // Budget 15 percent above a 128 kb/s MP3: 160 would need 25 percent more.
    let budget_128 = predict_bytes(AudioFormat::Mp3, 128_000, dur) * 115 / 100;
    let plan = plan_fit(dur, budget_128, &ALL, &caps, 2, None).unwrap();
    assert_eq!(plan.format, AudioFormat::Mp3);
    assert_eq!(plan.bitrate_bps, 128_000);
    assert_eq!(plan.channels, 2);
    assert_eq!(plan.sample_rate, 44_100);
    assert!(plan.predicted_bytes <= budget_128);
    assert_eq!(plan.attempt, 1);

    // Table: (budget as a multiple of 1 kB/s-equivalent, expected rung).
    let cases: [(u64, u32, u16, u32); 5] = [
        (
            predict_bytes(AudioFormat::Mp3, 192_000, dur) * 2,
            192_000,
            2,
            44_100,
        ),
        (
            predict_bytes(AudioFormat::Mp3, 96_000, dur) * 115 / 100,
            96_000,
            2,
            44_100,
        ),
        (
            predict_bytes(AudioFormat::Mp3, 80_000, dur) * 110 / 100,
            80_000,
            1,
            44_100,
        ),
        (
            predict_bytes(AudioFormat::Mp3, 64_000, dur) * 110 / 100,
            64_000,
            1,
            32_000,
        ),
        (
            predict_bytes(AudioFormat::Mp3, 64_000, dur) * 103 / 100,
            64_000,
            1,
            32_000,
        ),
    ];
    for (budget, bps, ch, sr) in cases {
        let p = plan_fit(dur, budget, &ALL, &caps, 2, None).unwrap();
        assert_eq!(
            (p.format, p.bitrate_bps, p.channels, p.sample_rate),
            (AudioFormat::Mp3, bps, ch, sr),
            "budget {budget}"
        );
        assert!(p.predicted_bytes <= budget);
    }

    // A mono source never gets a stereo plan.
    let p = plan_fit(dur, budget_128, &ALL, &caps, 1, None).unwrap();
    assert_eq!(p.channels, 1);

    // Preset order wins: Opus first when listed first.
    let p = plan_fit(dur, budget_128, &["ogg_opus", "mp3"], &caps, 2, None).unwrap();
    assert_eq!(p.format, AudioFormat::OggOpus);
    assert_eq!(p.bitrate_bps, 128_000);
    assert!(!p.opus_voip());

    // Host without FFmpeg skips MP3 and AAC.
    let p = plan_fit(
        dur,
        budget_128,
        &ALL,
        &HostAudioCaps::native_only(),
        2,
        None,
    )
    .unwrap();
    assert_eq!(p.format, AudioFormat::OggOpus);

    // Lossless when the estimate fits and FLAC is listed first.
    let p = plan_fit(
        dur,
        10_000_000,
        &["flac", "ogg_opus"],
        &caps,
        2,
        Some(6_000_000),
    )
    .unwrap();
    assert_eq!(p.format, AudioFormat::Flac);
    assert_eq!(p.predicted_bytes, 6_000_000);
    assert_eq!(p.channels, 2);
    // ... and falls through to Opus when it does not.
    let p = plan_fit(
        dur,
        1_000_000,
        &["flac", "ogg_opus"],
        &caps,
        2,
        Some(6_000_000),
    )
    .unwrap();
    assert_eq!(p.format, AudioFormat::OggOpus);
    assert_eq!(p.bitrate_bps, 128_000);

    // Opus below 32 kb/s uses VOIP.
    let tiny = predict_bytes(AudioFormat::OggOpus, 24_000, dur) * 105 / 100;
    let p = plan_fit(dur, tiny, &["ogg_opus"], &caps, 2, None).unwrap();
    assert_eq!((p.bitrate_bps, p.channels), (24_000, 1));
    assert!(p.opus_voip());
}

#[test]
fn plan_fit_refuses_with_a_max_duration_that_itself_fits() {
    let caps = HostAudioCaps::all();
    for budget in [50_000u64, 200_000, 1_000_000, 9_999_999] {
        for formats in [&ALL[..], &["mp3"][..], &["m4a_aac"][..], &["ogg_opus"][..]] {
            // 10 hours never fits.
            let err = plan_fit(36_000_000, budget, formats, &caps, 2, None).unwrap_err();
            let RefusalCode::TooLongForLimit { max_duration_ms } = err else {
                panic!("expected TooLongForLimit, got {err:?}");
            };
            assert!(max_duration_ms > 0, "budget {budget} {formats:?}");
            let p = plan_fit(max_duration_ms, budget, formats, &caps, 2, None)
                .unwrap_or_else(|e| panic!("max duration {max_duration_ms} must fit: {e:?}"));
            assert!(p.predicted_bytes <= budget * 102 / 100, "{p:?} vs {budget}");
            // One second more does not fit.
            assert!(
                plan_fit(max_duration_ms + 1_000, budget, formats, &caps, 2, None).is_err(),
                "budget {budget} {formats:?}: {} s should not fit",
                (max_duration_ms + 1000) / 1000
            );
        }
    }
    // Lossless-only presets refuse with max 0 (no lossy floor to compute).
    let err = plan_fit(600_000, 1_000_000, &["flac"], &caps, 2, Some(50_000_000)).unwrap_err();
    assert_eq!(err, RefusalCode::TooLongForLimit { max_duration_ms: 0 });
}

#[test]
fn retry_scale_shrinks_by_budget_over_actual_and_stops_after_four() {
    let dur = 60_000;
    let budget = predict_bytes(AudioFormat::OggOpus, 128_000, dur);
    let plan = plan_fit(
        dur,
        budget * 105 / 100,
        &["ogg_opus"],
        &HostAudioCaps::all(),
        2,
        None,
    )
    .unwrap();
    assert_eq!(plan.bitrate_bps, 128_000);

    // Fits: no retry.
    assert!(retry_scale(&plan, budget, budget).is_none());

    // 10 percent over: 128k * (1/1.1) * 0.97 = 112.87k.
    let r = retry_scale(&plan, budget * 110 / 100, budget).unwrap();
    assert_eq!(r.attempt, 2);
    assert!(
        (112_000..=113_000).contains(&r.bitrate_bps),
        "{}",
        r.bitrate_bps
    );
    assert_eq!(r.channels, 2);
    assert!(r.predicted_bytes < plan.predicted_bytes);

    // Falling through 32 kb/s makes it mono.
    let r2 = retry_scale(&r, budget * 4, budget).unwrap();
    assert!(
        r2.bitrate_bps < 32_000 && r2.bitrate_bps >= 16_000,
        "{}",
        r2.bitrate_bps
    );
    assert_eq!(r2.channels, 1);
    assert_eq!(r2.attempt, 3);

    // Below the floor: give up.
    assert!(retry_scale(&r2, budget * 10, budget).is_none());

    // Attempt cap.
    let mut p = plan;
    let mut n = 1;
    while let Some(next) = retry_scale(&p, budget * 101 / 100, budget) {
        p = next;
        n += 1;
        assert!(n <= MAX_ATTEMPTS, "must stop at {MAX_ATTEMPTS}");
    }
    assert_eq!(n, MAX_ATTEMPTS);

    // Lossless plans never retry.
    let flac = AudioPlan {
        format: AudioFormat::Flac,
        bitrate_bps: 800_000,
        channels: 2,
        sample_rate: 0,
        predicted_bytes: 6_000_000,
        attempt: 1,
    };
    assert!(retry_scale(&flac, 7_000_000, 6_000_000).is_none());
}

#[test]
fn plan_smaller_follows_the_design_paragraph() {
    let info = |codec: &str, lossless: bool, channels: u16, bitrate: Option<u64>| AudioInfo {
        codec: codec.into(),
        channels,
        sample_rate: 44_100,
        duration_ms: 10_000,
        bitrate_bps: bitrate,
        lossless,
        needs_ffmpeg: matches!(codec, "aac" | "alac" | "wma"),
    };
    use SmallerLevel::*;
    assert_eq!(
        plan_smaller(&info("pcm", true, 2, None), KeepQuality),
        SmallerDecision::ToFlac
    );
    assert_eq!(
        plan_smaller(&info("alac", true, 2, None), KeepQuality),
        SmallerDecision::ToFlac
    );
    assert_eq!(
        plan_smaller(&info("flac", true, 2, None), KeepQuality),
        SmallerDecision::ReencodeFlac
    );
    assert_eq!(
        plan_smaller(&info("mp3", false, 2, Some(320_000)), KeepQuality),
        SmallerDecision::KeepOriginal
    );
    assert_eq!(
        plan_smaller(&info("vorbis", false, 1, Some(110_000)), KeepQuality),
        SmallerDecision::KeepOriginal
    );
    assert_eq!(
        plan_smaller(&info("mp3", false, 2, Some(320_000)), Smallest),
        SmallerDecision::ToOpus {
            bps: 96_000,
            channels: 2
        }
    );
    assert_eq!(
        plan_smaller(&info("mp3", false, 1, Some(128_000)), Smallest),
        SmallerDecision::ToOpus {
            bps: 48_000,
            channels: 1
        }
    );
    assert_eq!(
        plan_smaller(&info("pcm", true, 2, Some(1_411_000)), Smallest),
        SmallerDecision::ToOpus {
            bps: 96_000,
            channels: 2
        }
    );
    // Already smaller than the Opus target: leave it.
    assert_eq!(
        plan_smaller(&info("opus", false, 2, Some(64_000)), Smallest),
        SmallerDecision::KeepOriginal
    );
}

#[test]
fn verify_passes_good_files_and_fails_truncated_ones() {
    let src = synth(10.0, 44_100, 2);
    let flac = encode_flac(&src, 8).unwrap();
    let ok = verify(&flac, 10_000, 2, Some(flac.len() as u64));
    assert!(ok.ok, "{ok:?}");
    assert_eq!(ok.duration_ms, Some(10_000));
    assert_eq!(ok.channels, Some(2));

    let over = verify(&flac, 10_000, 2, Some(flac.len() as u64 - 1));
    assert!(!over.ok && !over.size_ok && over.decodes);

    let wrong_ch = verify(&flac, 10_000, 1, None);
    assert!(!wrong_ch.ok && !wrong_ch.channels_ok);

    let wrong_dur = verify(&flac, 10_101, 2, None);
    assert!(!wrong_dur.ok && !wrong_dur.duration_ok);
    assert!(
        verify(&flac, 10_100, 2, None).ok,
        "100 ms is inside the tolerance"
    );

    let truncated = &flac[..flac.len() * 6 / 10];
    let bad = verify(truncated, 10_000, 2, None);
    assert!(!bad.ok, "{bad:?}");
    assert!(!bad.problems.is_empty());

    let wav = encode_wav(&src);
    assert!(verify(&wav, 10_000, 2, None).ok);
    assert!(!verify(&wav[..wav.len() / 2], 10_000, 2, None).ok);

    assert!(verify(MP3, 10_031, 1, None).ok);
    assert!(!verify(&MP3[..MP3.len() / 3], 10_031, 1, None).ok);
    assert!(verify(OGG_VORBIS, 10_000, 1, None).ok);
    assert!(!verify(&OGG_VORBIS[..OGG_VORBIS.len() / 2], 10_000, 1, None).ok);

    let garbage = verify(b"definitely not audio", 10_000, 2, None);
    assert!(!garbage.ok && !garbage.decodes);
}

#[cfg(feature = "opus-native")]
mod opus_native {
    use super::*;
    use cia_audio::{encode_opus_ogg, OpusApplication};

    #[test]
    fn opus_64k_stereo_lands_near_prediction_and_decodes_back() {
        let src = synth(10.0, 44_100, 2);
        let ogg = encode_opus_ogg(&src, 64_000, 2, OpusApplication::Audio).unwrap();
        let predicted = predict_bytes(AudioFormat::OggOpus, 64_000, 10_000);
        let ratio = ogg.len() as f64 / predicted as f64;
        eprintln!(
            "opus 64k stereo 10 s: {} bytes, predicted {predicted}, ratio {ratio:.3}",
            ogg.len()
        );
        assert!(
            (0.85..=1.15).contains(&ratio),
            "size {} vs predicted {predicted}",
            ogg.len()
        );
        assert_eq!(&ogg[0..4], b"OggS");
        assert_eq!(&ogg[28..36], b"OpusHead");

        let info = probe(&ogg).unwrap();
        assert_eq!(info.codec, "opus");
        assert_eq!(info.channels, 2);
        assert_eq!(info.sample_rate, 44_100, "OpusHead carries the input rate");
        assert!(
            info.duration_ms.abs_diff(10_000) <= 50,
            "{}",
            info.duration_ms
        );

        let back = decode(&ogg, None).unwrap();
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.channels, 2);
        assert!(
            back.duration_ms.abs_diff(10_000) <= 50,
            "{}",
            back.duration_ms
        );
        assert_eq!(back.source_codec, "opus");

        let report = verify(&ogg, 10_000, 2, Some(predicted * 115 / 100));
        assert!(report.ok, "{report:?}");
        assert!(!verify(&ogg[..ogg.len() / 2], 10_000, 2, None).ok);

        // Decoded content resembles the source: correlate the first second
        // after resampling the source to 48 kHz. Opus is not phase-exact, so
        // compare RMS envelopes instead.
        let ref48 = resample(&src, 48_000);
        let rms =
            |s: &[f32]| (s.iter().map(|v| (v * v) as f64).sum::<f64>() / s.len() as f64).sqrt();
        let (a, b) = (rms(&ref48.samples[..96_000]), rms(&back.samples[..96_000]));
        assert!((a - b).abs() / a < 0.1, "rms {a} vs {b}");

        // Mono VOIP at the floor, and a mono source upmixed to stereo.
        let voip = encode_opus_ogg(&src, 16_000, 1, OpusApplication::Voip).unwrap();
        let d = decode(&voip, None).unwrap();
        assert_eq!(d.channels, 1);
        assert!(d.duration_ms.abs_diff(10_000) <= 50);
        let mono = downmix_mono(&src);
        let up = encode_opus_ogg(&mono, 48_000, 2, OpusApplication::Audio).unwrap();
        assert_eq!(decode(&up, None).unwrap().channels, 2);

        // Prediction accuracy across the ladder (informational, loose bound).
        for bps in [128_000u32, 96_000, 48_000, 32_000, 24_000] {
            let ch = if bps >= 48_000 { 2 } else { 1 };
            let app = if bps < 32_000 {
                OpusApplication::Voip
            } else {
                OpusApplication::Audio
            };
            let out = encode_opus_ogg(&src, bps, ch, app).unwrap();
            let pred = predict_bytes(AudioFormat::OggOpus, bps, 10_000);
            eprintln!(
                "opus {bps} {ch}ch: {} bytes, predicted {pred}, ratio {:.3}",
                out.len(),
                out.len() as f64 / pred as f64
            );
            assert!(
                (0.75..=1.25).contains(&(out.len() as f64 / pred as f64)),
                "{bps}: {} vs {pred}",
                out.len()
            );
        }

        assert!(encode_opus_ogg(&src, 64_000, 6, OpusApplication::Audio).is_err());
        assert!(encode_opus_ogg(&src, 1_000, 2, OpusApplication::Audio).is_err());
    }

    #[test]
    fn ffprobe_accepts_our_ogg_opus() {
        let Ok(out) = std::process::Command::new("which").arg("ffprobe").output() else {
            return;
        };
        if !out.status.success() {
            eprintln!("ffprobe not on PATH, skipping");
            return;
        }
        let src = synth(10.0, 44_100, 2);
        let ogg = encode_opus_ogg(&src, 64_000, 2, OpusApplication::Audio).unwrap();
        let path = std::env::temp_dir().join(format!("cia-audio-{}.ogg", std::process::id()));
        std::fs::write(&path, &ogg).unwrap();

        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,channels,sample_rate:format=duration",
                "-of",
                "flat",
            ])
            .arg(&path)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&probe.stdout);
        let stderr = String::from_utf8_lossy(&probe.stderr);
        assert!(probe.status.success(), "ffprobe failed: {stderr}");
        assert!(
            stderr.trim().is_empty(),
            "ffprobe reported errors: {stderr}"
        );
        assert!(stdout.contains("codec_name=\"opus\""), "{stdout}");
        assert!(stdout.contains("channels=2"), "{stdout}");
        let dur: f64 = stdout
            .lines()
            .find_map(|l| l.strip_prefix("format.duration=\""))
            .and_then(|v| v.trim_end_matches('"').parse().ok())
            .expect("duration");
        assert!((dur - 10.0).abs() <= 0.05, "ffprobe duration {dur}");

        // A full decode with FFmpeg must be clean too.
        let dec = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "null", "-"])
            .output();
        if let Ok(dec) = dec {
            let err = String::from_utf8_lossy(&dec.stderr);
            assert!(
                dec.status.success() && err.trim().is_empty(),
                "ffmpeg decode: {err}"
            );
        }

        // FFmpeg must also read our FLAC (the STREAMINFO block-size fix).
        let flac_path = path.with_extension("flac");
        std::fs::write(&flac_path, encode_flac(&src, 8).unwrap()).unwrap();
        let fl = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,channels:format=duration",
                "-of",
                "flat",
            ])
            .arg(&flac_path)
            .output()
            .unwrap();
        let fl_out = String::from_utf8_lossy(&fl.stdout);
        assert!(
            fl.status.success() && fl_out.contains("codec_name=\"flac\""),
            "{fl_out}"
        );
        assert!(fl_out.contains("format.duration=\"10.000000\""), "{fl_out}");
        let _ = std::fs::remove_file(&flac_path);

        // opusinfo, when installed, must not complain either.
        if let Ok(oi) = std::process::Command::new("opusinfo").arg(&path).output() {
            let text = String::from_utf8_lossy(&oi.stdout) + String::from_utf8_lossy(&oi.stderr);
            assert!(oi.status.success(), "opusinfo: {text}");
            assert!(!text.to_lowercase().contains("warning"), "opusinfo: {text}");
        }
        let _ = std::fs::remove_file(&path);
    }
}
