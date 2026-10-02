#![allow(clippy::type_complexity)] // table-driven tests use wide tuples on purpose
//! Table-driven tests for DESIGN.md 3.5 with the numbers written out (7.1).

use cia_core::presets::{self, VideoCaps, VideoFormat};
use cia_core::{AudioTrackChoice, FrameRatePref, QualityLabel, RefusalCode, Suggestion};
use cia_video_plan::*;

const DISCORD_FREE: Budget = Budget {
    raw_budget_bytes: 20_905_984,
    hard_bytes: 20_971_520,
    safety_bytes: 65_536,
};

fn discord_caps() -> VideoCaps {
    VideoCaps {
        max_short_edge: Some(1080),
        max_fps: Some(60),
    }
}

fn whatsapp_caps() -> VideoCaps {
    VideoCaps {
        max_short_edge: Some(720),
        max_fps: Some(30),
    }
}

fn h264_mp4(caps: VideoCaps) -> Target {
    Target::new(Container::Mp4, VideoCodec::H264, AudioCodec::Aac).with_caps(caps)
}

fn track(index: u32, channels: u32, title: &str) -> AudioTrack {
    AudioTrack {
        index,
        codec: "aac".into(),
        channels,
        sample_rate: 48_000,
        bitrate_bps: Some(160_000),
        title: Some(title.into()),
    }
}

fn probe(w: u32, h: u32, fps: f32, duration_ms: u64, audio: Vec<AudioTrack>) -> VideoProbe {
    VideoProbe {
        duration_ms,
        container: "mkv".into(),
        video_codec: "h264".into(),
        coded_w: w,
        coded_h: h,
        display_w: w,
        display_h: h,
        avg_fps: fps,
        max_fps: fps,
        pixel_format: "yuv420p".into(),
        audio,
        ..Default::default()
    }
}

/// Jayden's OBS clip (DESIGN 1.3): 1440p60 MKV, game and mic tracks.
fn obs_clip(duration_ms: u64) -> VideoProbe {
    probe(
        2560,
        1440,
        60.0,
        duration_ms,
        vec![track(0, 2, "Game"), track(1, 2, "Mic")],
    )
}

fn rungs(list: &[(u32, u32, f32)]) -> Vec<Rung> {
    list.iter()
        .map(|&(width, height, fps)| Rung { width, height, fps })
        .collect()
}

fn fmt(container: &str, video: &str, audio: &str) -> VideoFormat {
    VideoFormat {
        container: container.into(),
        video: video.into(),
        audio: audio.into(),
    }
}

// ---------------------------------------------------------------- 3.5.4 worked example

#[test]
fn worked_example_discord_free_nvenc() {
    // 70 s 1440p60, Discord Free raw budget 20,905,984, single-pass hardware (margin 0.92).
    let p = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &h264_mp4(discord_caps()),
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap();
    // overhead = 4096 + 70 * (30 * 14 + 1 * 470) = 66,396
    assert_eq!(p.overhead_bytes, 66_396);
    // total_bps = floor((20,905,984 - 66,396) * 8 * 0.92 / 70) = floor(2,191,133.8) = 2,191,133
    assert_eq!(p.total_bps(), 2_191_133);
    // 15 percent is 328,669, so AAC 128 kb/s stereo
    assert_eq!((p.audio_bps, p.audio_channels), (128_000, 2));
    assert_eq!(p.video_bps, 2_063_133);
    // Discord caps the short edge at 1080, so the ladder starts at 1080p60.
    assert_eq!(
        p.ladder,
        rungs(&[
            (1920, 1080, 60.0),
            (1920, 1080, 30.0),
            (1280, 720, 30.0),
            (960, 540, 30.0),
            (854, 480, 30.0),
            (640, 360, 30.0),
        ])
    );
    // 720p30: 2,063,133 / (1280 * 720 * 30) = 0.0746 >= 0.050; 0.0746 / 0.050 = 1.49, Good
    assert_eq!(
        (p.width, p.height, p.fps, p.rung_index),
        (1280, 720, 30.0, 2)
    );
    assert!((p.bpp - 0.074_622).abs() < 1e-6, "bpp {}", p.bpp);
    assert_eq!(p.floor_bpp, 0.050);
    assert_eq!(p.quality, QualityLabel::Good);
    assert!(!p.two_pass);
    // predicted = ceil(2,191,133 * 70 / 8) + 66,396 = 19,172,414 + 66,396
    assert_eq!(p.predicted_bytes, 19_238_810);
    assert_eq!(cia_core::format::mb(p.predicted_bytes), "19.2 MB");
    assert!(p.predicted_bytes <= DISCORD_FREE.raw_budget_bytes);
    assert_eq!(
        (p.container, p.video_codec, p.audio_codec),
        (Container::Mp4, VideoCodec::H264, AudioCodec::Aac)
    );
}

#[test]
fn worked_example_rung_bpps_match_design() {
    // The bpp the design quotes per rung: 1440p60 0.0093, 1440p30 0.019, 1080p30 0.033.
    // Without the Discord short-edge cap the ladder keeps the 1440 rungs.
    let no_caps = h264_mp4(VideoCaps::default());
    let p = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &no_caps,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap();
    assert_eq!(
        p.ladder[0],
        Rung {
            width: 2560,
            height: 1440,
            fps: 60.0
        }
    );
    assert_eq!(
        p.ladder[1],
        Rung {
            width: 2560,
            height: 1440,
            fps: 30.0
        }
    );
    assert_eq!(
        p.ladder[2],
        Rung {
            width: 1920,
            height: 1080,
            fps: 30.0
        }
    );
    assert_eq!((p.width, p.height, p.rung_index), (1280, 720, 3));
    // 60 fps rung pays its own overhead: 4096 + 70 * (60 * 14 + 470) = 95,796;
    // total = floor((20,905,984 - 95,796) * 7.36 / 70) = 2,188,042; video = 2,060,042
    let bpp_1440p60: f64 = 2_060_042.0 / (2560.0 * 1440.0 * 60.0);
    assert!((bpp_1440p60 - 0.0093).abs() < 0.0001);
    let bpp_1440p30: f64 = 2_063_133.0 / (2560.0 * 1440.0 * 30.0);
    assert!((bpp_1440p30 - 0.019).abs() < 0.001);
    let bpp_1080p30: f64 = 2_063_133.0 / (1920.0 * 1080.0 * 30.0);
    assert!((bpp_1080p30 - 0.033).abs() < 0.001);
}

#[test]
fn worked_example_two_pass_software() {
    // Same clip with libx264 (margin 0.96): total = floor(20,839,588 * 7.68 / 70) = 2,286,400.
    let p = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &h264_mp4(discord_caps()),
        EncoderKind::TwoPassSoftware,
        &PlanOptions::default(),
    )
    .unwrap();
    assert_eq!(p.total_bps(), 2_286_400);
    assert_eq!(p.video_bps, 2_158_400);
    assert_eq!((p.width, p.height, p.fps), (1280, 720, 30.0));
    assert!(p.two_pass);
    assert_eq!(p.quality, QualityLabel::Good); // 0.0781 / 0.050 = 1.56
    assert_eq!(p.predicted_bytes, 20_072_396);
}

#[test]
fn two_minutes_four_seconds_drops_to_540p() {
    // The brief quoted "2 min 4 s -> 720p, about 19.6 MB, Good"; the 3.5.4 formula gives 540p.
    // overhead = 4096 + 124 * 890 = 114,456; total = floor(20,791,528 * 7.36 / 124) = 1,234,077
    // audio 128k; video 1,106,077; 720p bpp 0.0400 < 0.050; 540p bpp 0.0711 >= 0.060, Okay.
    let p = plan(
        &obs_clip(124_000),
        &DISCORD_FREE,
        &h264_mp4(discord_caps()),
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap();
    assert_eq!(p.overhead_bytes, 114_456);
    assert_eq!(p.total_bps(), 1_234_077);
    assert_eq!(p.video_bps, 1_106_077);
    assert_eq!(
        (p.width, p.height, p.fps, p.rung_index),
        (960, 540, 30.0, 3)
    );
    assert_eq!(p.quality, QualityLabel::Okay);
    assert_eq!(p.predicted_bytes, 19_242_650);
    assert_eq!(cia_core::format::mb(p.predicted_bytes), "19.2 MB");
}

#[test]
fn trim_plans_the_trimmed_range() {
    let opts = PlanOptions {
        trim: Some((10_000, 80_000)),
        ..Default::default()
    };
    let p = plan(
        &obs_clip(600_000),
        &DISCORD_FREE,
        &h264_mp4(discord_caps()),
        EncoderKind::SinglePassHardware,
        &opts,
    )
    .unwrap();
    assert_eq!(p.duration_ms, 70_000);
    assert_eq!(p.predicted_bytes, 19_238_810);
}

#[test]
fn predict_for_preset_matches_manual_budget() {
    let preset = presets::find("discord-free").unwrap();
    let p =
        predict_for_preset(&obs_clip(70_000), &preset, EncoderKind::SinglePassHardware).unwrap();
    assert_eq!(
        (p.width, p.height, p.predicted_bytes),
        (1280, 720, 19_238_810)
    );
    let b = Budget::from_resolved(&preset.resolve().unwrap());
    assert_eq!(b, DISCORD_FREE);
}

// ---------------------------------------------------------------- 3.5.3 audio

#[test]
fn audio_fifteen_percent_cap() {
    use AudioCodec::*;
    let mix = AudioTrackChoice::MixAll;
    // (total_bps, codec, source channels) -> (bps, channels)
    let table: &[(u64, AudioCodec, u32, (u64, u32))] = &[
        (2_191_133, Aac, 2, (128_000, 2)), // 15 % = 328,669
        (853_334, Aac, 2, (128_000, 2)),   // 15 % = 128,000 exactly
        (853_333, Aac, 2, (96_000, 2)),    // 15 % = 127,999
        (640_000, Aac, 2, (96_000, 2)),    // 96,000
        (639_999, Aac, 2, (64_000, 2)),    // 95,999
        (426_667, Aac, 2, (64_000, 2)),    // 64,000
        (426_666, Aac, 2, (48_000, 1)),    // 63,999
        (320_000, Aac, 2, (48_000, 1)),    // 48,000
        (319_999, Aac, 2, (32_000, 1)),    // 47,999
        (213_334, Aac, 2, (32_000, 1)),    // 32,000
        (100_000, Aac, 2, (32_000, 1)),    // 15,000: nothing fits, lowest rung
        (0, Aac, 2, (32_000, 1)),
        (2_191_133, Aac, 1, (128_000, 1)), // mono never upmixes
        (2_191_133, Aac, 6, (128_000, 2)), // 5.1 downmixes to the rung's stereo
        (640_000, Opus, 2, (96_000, 2)),
        (426_666, Opus, 2, (48_000, 2)), // Opus 48 is stereo
        (319_999, Opus, 2, (32_000, 1)),
        (100_000, Opus, 2, (24_000, 1)), // lowest Opus rung
        (2_191_133, Aac, 0, (0, 0)),     // no audio in the source
    ];
    for &(total, codec, channels, expect) in table {
        let a = pick_audio(total, codec, channels, &mix);
        assert_eq!(
            (a.bps, a.channels),
            expect,
            "total {total} {codec:?} ch {channels}"
        );
    }
    let r = pick_audio(2_191_133, Aac, 2, &AudioTrackChoice::Remove);
    assert_eq!(r, AudioChoice::NONE);
    let t = pick_audio(2_191_133, Aac, 2, &AudioTrackChoice::Track { index: 1 });
    assert_eq!((t.bps, t.channels), (128_000, 2));
}

#[test]
fn audio_ladders_are_the_design_tables() {
    let kb = |l: &[AudioRung]| {
        l.iter()
            .map(|r| (r.bps / 1000, r.channels))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kb(video_audio_ladder(AudioCodec::Aac)),
        [(128, 2), (96, 2), (64, 2), (48, 1), (32, 1)]
    );
    assert_eq!(
        kb(video_audio_ladder(AudioCodec::Opus)),
        [(96, 2), (64, 2), (48, 2), (32, 1), (24, 1)]
    );
    assert_eq!(
        kb(standalone_ladder(StandaloneAudioFormat::Mp3)),
        [
            (192, 2),
            (160, 2),
            (128, 2),
            (112, 2),
            (96, 2),
            (80, 1),
            (64, 1)
        ]
    );
    assert_eq!(
        kb(standalone_ladder(StandaloneAudioFormat::Aac)),
        [
            (160, 2),
            (128, 2),
            (96, 2),
            (80, 2),
            (64, 2),
            (48, 1),
            (32, 1)
        ]
    );
    assert_eq!(
        kb(standalone_ladder(StandaloneAudioFormat::Opus)),
        [
            (128, 2),
            (96, 2),
            (64, 2),
            (48, 2),
            (32, 1),
            (24, 1),
            (16, 1)
        ]
    );
}

#[test]
fn audio_choices_in_a_plan() {
    let target = h264_mp4(discord_caps());
    let mic_only = PlanOptions {
        audio: AudioTrackChoice::Track { index: 1 },
        ..Default::default()
    };
    let mut clip = obs_clip(70_000);
    clip.audio[1].channels = 1;
    let p = plan(
        &clip,
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &mic_only,
    )
    .unwrap();
    assert_eq!((p.audio_bps, p.audio_channels), (128_000, 1));

    let removed = PlanOptions {
        audio: AudioTrackChoice::Remove,
        ..Default::default()
    };
    let p = plan(
        &clip,
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &removed,
    )
    .unwrap();
    assert_eq!((p.audio_bps, p.audio_channels), (0, 0));
    // No audio track out: overhead = 4096 + 70 * 420 = 33,496; total = floor(20,872,488 * 7.36 / 70)
    assert_eq!(p.overhead_bytes, 33_496);
    assert_eq!(p.video_bps, 2_194_593);
    assert_eq!(p.total_bps(), p.video_bps);
}

// ---------------------------------------------------------------- 3.5.4 ladder

#[test]
fn ladder_tables() {
    let auto = FrameRatePref::Automatic;
    let none = VideoCaps::default();
    let table: Vec<(&str, VideoProbe, VideoCaps, FrameRatePref, Vec<Rung>)> = vec![
        (
            "portrait phone 1080x1920 steps on the short side, even sizes",
            probe(1080, 1920, 30.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (1080, 1920, 30.0),
                (720, 1280, 30.0),
                (540, 960, 30.0),
                (480, 854, 30.0),
                (360, 640, 30.0),
            ]),
        ),
        (
            "landscape 1080p60: 60 -> 30 rung first, then heights at 30",
            probe(1920, 1080, 60.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (1920, 1080, 60.0),
                (1920, 1080, 30.0),
                (1280, 720, 30.0),
                (960, 540, 30.0),
                (854, 480, 30.0),
                (640, 360, 30.0),
            ]),
        ),
        (
            "never upscale: 640x360 source has one rung",
            probe(640, 360, 30.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[(640, 360, 30.0)]),
        ),
        (
            "never upscale: 854x480 60 fps keeps only rungs below it",
            probe(854, 480, 60.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[(854, 480, 60.0), (854, 480, 30.0), (640, 360, 30.0)]),
        ),
        (
            "whatsapp max_short_edge 720 and max_fps 30 on a 1080p60 source",
            probe(1920, 1080, 60.0, 10_000, vec![]),
            whatsapp_caps(),
            auto.clone(),
            rungs(&[
                (1280, 720, 30.0),
                (960, 540, 30.0),
                (854, 480, 30.0),
                (640, 360, 30.0),
            ]),
        ),
        (
            "whatsapp on a portrait 1080x1920 60 fps source",
            probe(1080, 1920, 60.0, 10_000, vec![]),
            whatsapp_caps(),
            auto.clone(),
            rungs(&[
                (720, 1280, 30.0),
                (540, 960, 30.0),
                (480, 854, 30.0),
                (360, 640, 30.0),
            ]),
        ),
        (
            "discord caps a 1440p60 source to 1080p but keeps 60 fps",
            probe(2560, 1440, 60.0, 10_000, vec![]),
            discord_caps(),
            auto.clone(),
            rungs(&[
                (1920, 1080, 60.0),
                (1920, 1080, 30.0),
                (1280, 720, 30.0),
                (960, 540, 30.0),
                (854, 480, 30.0),
                (640, 360, 30.0),
            ]),
        ),
        (
            "120 fps source is capped at 60",
            probe(1920, 1080, 120.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (1920, 1080, 60.0),
                (1920, 1080, 30.0),
                (1280, 720, 30.0),
                (960, 540, 30.0),
                (854, 480, 30.0),
                (640, 360, 30.0),
            ]),
        ),
        (
            "24 fps source stays 24 and gets no 30 fps rung",
            probe(1920, 1080, 24.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (1920, 1080, 24.0),
                (1280, 720, 24.0),
                (960, 540, 24.0),
                (854, 480, 24.0),
                (640, 360, 24.0),
            ]),
        ),
        (
            "keep original frame rate: no 30 fps rung, 60 all the way down",
            probe(1920, 1080, 60.0, 10_000, vec![]),
            none.clone(),
            FrameRatePref::KeepOriginal,
            rungs(&[
                (1920, 1080, 60.0),
                (1280, 720, 60.0),
                (960, 540, 60.0),
                (854, 480, 60.0),
                (640, 360, 60.0),
            ]),
        ),
        (
            "30 fps preference starts at 30",
            probe(1920, 1080, 60.0, 10_000, vec![]),
            none.clone(),
            FrameRatePref::Fps30,
            rungs(&[
                (1920, 1080, 30.0),
                (1280, 720, 30.0),
                (960, 540, 30.0),
                (854, 480, 30.0),
                (640, 360, 30.0),
            ]),
        ),
        (
            "odd source size rounds down to even, never up; lower rungs scale from the real aspect",
            probe(1081, 1921, 30.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (1080, 1920, 30.0),
                (720, 1280, 30.0), // 1921 * 720 / 1081 = 1279.5 -> 1280
                (540, 960, 30.0),  // 959.6 -> 960
                (480, 852, 30.0),  // 852.99 -> nearest even 852
                (360, 640, 30.0),  // 639.7 -> 640
            ]),
        ),
        (
            "ultrawide 2560x1080 keeps aspect, nearest even width like scale=-2:H",
            probe(2560, 1080, 30.0, 10_000, vec![]),
            none.clone(),
            auto.clone(),
            rungs(&[
                (2560, 1080, 30.0),
                (1706, 720, 30.0),
                (1280, 540, 30.0),
                (1138, 480, 30.0),
                (854, 360, 30.0),
            ]),
        ),
    ];
    for (name, probe, caps, pref, expect) in table {
        let got = build_rungs(&probe, &caps, &pref);
        assert_eq!(got, expect, "{name}");
        for r in &got {
            assert_eq!(r.width % 2, 0, "{name}: odd width {}", r.width);
            assert_eq!(r.height % 2, 0, "{name}: odd height {}", r.height);
            assert!(
                r.width <= probe.display_w && r.height <= probe.display_h,
                "{name}: upscale"
            );
            assert!(r.fps <= 60.0, "{name}: fps {}", r.fps);
        }
    }
}

#[test]
fn floors_and_labels_table() {
    let table: &[(VideoCodec, u32, f64)] = &[
        (VideoCodec::H264, 1080, 0.050),
        (VideoCodec::H264, 720, 0.050),
        (VideoCodec::H264, 540, 0.060),
        (VideoCodec::H264, 480, 0.060),
        (VideoCodec::H264, 360, 0.070),
        (VideoCodec::Vp9, 720, 0.0375),
        (VideoCodec::Vp9, 360, 0.0525),
        (VideoCodec::Av1, 720, 0.030),
        (VideoCodec::Av1, 540, 0.036),
    ];
    for &(codec, h, floor) in table {
        assert!((floor_bpp(codec, h) - floor).abs() < 1e-12, "{codec:?} {h}");
    }
    assert_eq!(quality_label(0.1101, 0.050), QualityLabel::Great); // 2.2x
    assert_eq!(quality_label(0.1099, 0.050), QualityLabel::Good);
    assert_eq!(quality_label(0.0701, 0.050), QualityLabel::Good); // 1.4x
    assert_eq!(quality_label(0.0699, 0.050), QualityLabel::Okay);
}

#[test]
fn vp9_webm_target_uses_opus_and_webm_overhead() {
    let target =
        Target::new(Container::Webm, VideoCodec::Vp9, AudioCodec::Opus).with_caps(discord_caps());
    let p = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::TwoPassSoftware,
        &PlanOptions::default(),
    )
    .unwrap();
    // overhead = 4096 + 70 * (30 * 12 + 400) = 57,296; total = floor(20,848,688 * 7.68 / 70) = 2,287,398
    assert_eq!(p.overhead_bytes, 57_296);
    assert_eq!(p.total_bps(), 2_287_398);
    assert_eq!((p.audio_bps, p.audio_channels), (96_000, 2));
    // video 2,191,398 at 1080p30: bpp 0.0352 < 0.0375; at 720p30: 0.0793 >= 0.0375, 2.11x -> Good
    assert_eq!((p.width, p.height, p.fps), (1280, 720, 30.0));
    assert_eq!(p.quality, QualityLabel::Good);
}

// ---------------------------------------------------------------- 3.5.5 refusal

#[test]
fn refusal_max_duration_fits_and_one_second_more_refuses() {
    let target = h264_mp4(discord_caps());
    for (encoder, expect_max_ms) in [
        (EncoderKind::SinglePassHardware, 277_000),
        (EncoderKind::TwoPassSoftware, 289_000),
    ] {
        let err = plan(
            &obs_clip(600_000),
            &DISCORD_FREE,
            &target,
            encoder,
            &PlanOptions::default(),
        )
        .unwrap_err();
        let max_ms = err.max_duration_ms().expect("TooLongForLimit");
        assert_eq!(max_ms, expect_max_ms, "{encoder:?}");
        assert_eq!(
            err.code,
            RefusalCode::TooLongForLimit {
                max_duration_ms: max_ms
            }
        );
        assert_eq!(
            err.suggestions,
            vec![Suggestion::Trim {
                max_duration_ms: max_ms
            }]
        );

        let fits = plan(
            &obs_clip(max_ms),
            &DISCORD_FREE,
            &target,
            encoder,
            &PlanOptions::default(),
        )
        .unwrap();
        assert_eq!(
            (fits.width, fits.height, fits.fps),
            (640, 360, 30.0),
            "{encoder:?}"
        );
        assert!(fits.bpp >= 0.070);
        assert!(fits.predicted_bytes <= DISCORD_FREE.raw_budget_bytes);
        let over = plan(
            &obs_clip(max_ms + 1000),
            &DISCORD_FREE,
            &target,
            encoder,
            &PlanOptions::default(),
        );
        assert!(over.is_err(), "{encoder:?}: {max_ms} + 1 s should refuse");
    }
    // Hardware at 277 s: overhead 250,626; total = floor(20,655,358 * 7.36 / 277) = 548,821;
    // 15 % = 82,323 -> 64 kb/s stereo; video 484,821 / (640 * 360 * 30) = 0.07014 >= 0.070.
    let p = plan(
        &obs_clip(277_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap();
    assert_eq!(p.overhead_bytes, 250_626);
    assert_eq!(p.total_bps(), 548_821);
    assert_eq!((p.audio_bps, p.video_bps), (64_000, 484_821));
    assert_eq!(p.quality, QualityLabel::Okay);
    // The design's "4 min 54 s drops to 360p" assumes 32 kb/s audio; the 15 percent rule picks
    // 64 kb/s there and the video falls under the floor (docs/DECISIONS.md).
    assert!(plan(
        &obs_clip(294_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default()
    )
    .is_err());
}

#[test]
fn refusal_without_audio_and_pick_preset_suggestions() {
    let target = h264_mp4(discord_caps());
    let removed = PlanOptions {
        audio: AudioTrackChoice::Remove,
        ..Default::default()
    };
    let err = plan(
        &obs_clip(600_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &removed,
    )
    .unwrap_err();
    let max_ms = err.max_duration_ms().unwrap();
    // min video = 0.070 * 640 * 360 * 30 = 483,840; overhead = 4096 + 420 * d (no audio track);
    // (20,901,888 - 420 d) * 7.36 >= 483,840 d  ->  d <= 315.93
    assert_eq!(max_ms, 315_000);
    assert!(plan(
        &obs_clip(max_ms),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &removed
    )
    .is_ok());
    assert!(plan(
        &obs_clip(max_ms + 1000),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &removed
    )
    .is_err());

    let presets = presets::all();
    let s = suggest_presets(
        &obs_clip(600_000),
        &presets,
        EncoderKind::SinglePassHardware,
        "discord-free",
    );
    let ids: Vec<&str> = s
        .iter()
        .map(|s| match s {
            Suggestion::PickPreset { preset_id, .. } => preset_id.as_str(),
            _ => panic!(),
        })
        .collect();
    assert!(ids.contains(&"discord-nitro-basic"));
    assert!(!ids.contains(&"discord-free"));
    assert!(!ids.contains(&"custom"), "no limit, no suggestion");
    let nitro_basic = s
        .iter()
        .find_map(|s| match s {
            Suggestion::PickPreset {
                preset_id,
                predicted_bytes,
            } if preset_id == "discord-nitro-basic" => Some(*predicted_bytes),
            _ => None,
        })
        .unwrap();
    assert!(nitro_basic <= 50_000_000 - 65_536, "{nitro_basic}");
}

#[test]
fn refusal_codes_for_degenerate_input() {
    let target = h264_mp4(discord_caps());
    let mut p = obs_clip(0);
    let err = plan(
        &p,
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err.code, RefusalCode::UnsupportedInput { .. }));
    p.duration_ms = 70_000;
    p.display_w = 0;
    let err = plan(
        &p,
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err.code, RefusalCode::UnsupportedInput { .. }));
    // A budget too small for even one second: no Trim suggestion.
    let tiny = Budget {
        raw_budget_bytes: 10_000,
        hard_bytes: 10_000,
        safety_bytes: 0,
    };
    let err = plan(
        &obs_clip(70_000),
        &tiny,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap_err();
    assert_eq!(err.code, RefusalCode::BelowQualityFloor);
    assert!(err.suggestions.is_empty());
}

// ---------------------------------------------------------------- 3.5.8 retry

#[test]
fn retry_scaling_software_vs_hardware() {
    let target = h264_mp4(discord_caps());
    let hw = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::SinglePassHardware,
        &PlanOptions::default(),
    )
    .unwrap();
    let sw = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &target,
        EncoderKind::TwoPassSoftware,
        &PlanOptions::default(),
    )
    .unwrap();

    // Hardware came out at 21.3 MB: factor = 20,905,984 / 21,300,000 * 0.93 = 0.91280
    // video = floor(2,063,133 * 0.91280) = 1,883,220; 720p bpp 0.0681 >= 0.050, 1.36x -> Okay
    let r = retry_scale(
        &hw,
        21_300_000,
        &DISCORD_FREE,
        EncoderKind::SinglePassHardware,
    )
    .unwrap();
    assert_eq!(
        (r.width, r.height, r.fps, r.rung_index),
        (1280, 720, 30.0, 2)
    );
    assert_eq!(r.video_bps, 1_883_220);
    assert_eq!(r.audio_bps, 128_000);
    assert_eq!(r.quality, QualityLabel::Okay);
    // predicted = ceil(2,011,220 * 70 / 8) + 66,396 = 17,598,175 + 66,396
    assert_eq!(r.predicted_bytes, 17_664_571);

    // Software at 21.3 MB: factor = 0.98150 * 0.97 = 0.95206; video = floor(2,158,400 * 0.95206) = 2,054,918
    let r = retry_scale(&sw, 21_300_000, &DISCORD_FREE, EncoderKind::TwoPassSoftware).unwrap();
    assert_eq!(r.video_bps, 2_054_918);
    assert_eq!((r.width, r.height), (1280, 720));
    assert_eq!(r.quality, QualityLabel::Good); // 0.0743 / 0.050 = 1.49
    assert_eq!(r.predicted_bytes, 19_166_929);
    assert!(r.two_pass);

    // 30 MB out of hardware: factor 0.64809; video 1,337,086 < 720p floor 1,382,400 -> 540p30,
    // bpp 0.0860 >= 0.060, 1.43x -> Good
    let r = retry_scale(
        &hw,
        30_000_000,
        &DISCORD_FREE,
        EncoderKind::SinglePassHardware,
    )
    .unwrap();
    assert_eq!(
        (r.width, r.height, r.fps, r.rung_index),
        (960, 540, 30.0, 3)
    );
    assert_eq!(r.video_bps, 1_337_086);
    assert_eq!(r.quality, QualityLabel::Good);
    assert_eq!(r.predicted_bytes, 12_885_899);

    // 100 MB: 1,000,000-ish bps meets no floor down to 360p -> no rung remains
    assert!(retry_scale(
        &hw,
        100_000_000,
        &DISCORD_FREE,
        EncoderKind::SinglePassHardware
    )
    .is_none());

    // Under budget: factor is clamped to 1, nothing grows (undershoot is accepted as is).
    let r = retry_scale(
        &hw,
        15_000_000,
        &DISCORD_FREE,
        EncoderKind::SinglePassHardware,
    )
    .unwrap();
    assert_eq!(r.video_bps, hw.video_bps);
    assert_eq!(UNDERSHOOT_ACCEPT_FRACTION, 0.85);
}

// ---------------------------------------------------------------- keep / remux / encode

#[test]
fn keep_original_remux_or_encode() {
    let discord = [fmt("mp4", "h264", "aac"), fmt("webm", "vp9", "opus")];
    let whatsapp = [fmt("mp4", "h264", "aac")];
    let one_aac = vec![track(0, 2, "")];
    let src = |container: &str, codec: &str, audio: Vec<AudioTrack>| VideoProbe {
        container: container.into(),
        video_codec: codec.into(),
        audio,
        ..probe(1920, 1080, 30.0, 10_000, vec![])
    };
    let opus = vec![AudioTrack {
        codec: "opus".into(),
        ..track(0, 2, "")
    }];
    let table: Vec<(&str, VideoProbe, u64, u64, &[VideoFormat], Decision)> = vec![
        (
            "mp4 h264+aac that fits: keep",
            src("mp4", "h264", one_aac.clone()),
            20_905_984,
            10_000_000,
            &discord,
            Decision::KeepOriginal,
        ),
        (
            "exactly the budget still fits",
            src("mp4", "h264", one_aac.clone()),
            20_905_984,
            20_905_984,
            &discord,
            Decision::KeepOriginal,
        ),
        (
            "one byte over: encode",
            src("mp4", "h264", one_aac.clone()),
            20_905_984,
            20_905_985,
            &discord,
            Decision::Encode,
        ),
        (
            "mkv h264+aac for whatsapp: remux to mp4",
            src("mkv", "h264", one_aac.clone()),
            63_000_000,
            30_000_000,
            &whatsapp,
            Decision::Remux {
                container: Container::Mp4,
            },
        ),
        (
            "mov h264+aac: remux to mp4",
            src("mov", "h264", one_aac.clone()),
            63_000_000,
            30_000_000,
            &whatsapp,
            Decision::Remux {
                container: Container::Mp4,
            },
        ),
        (
            "mkv hevc: encode",
            src("mkv", "hevc", one_aac.clone()),
            63_000_000,
            30_000_000,
            &whatsapp,
            Decision::Encode,
        ),
        (
            "webm vp9+opus fits discord: keep",
            src("webm", "vp9", opus.clone()),
            20_905_984,
            10_000_000,
            &discord,
            Decision::KeepOriginal,
        ),
        (
            "mkv vp9+opus for discord: remux to webm",
            src("mkv", "vp9", opus.clone()),
            20_905_984,
            10_000_000,
            &discord,
            Decision::Remux {
                container: Container::Webm,
            },
        ),
        (
            "webm vp9+opus for whatsapp: encode",
            src("webm", "vp9", opus),
            63_000_000,
            10_000_000,
            &whatsapp,
            Decision::Encode,
        ),
        (
            "h264 with opus audio in mp4: encode",
            src(
                "mp4",
                "h264",
                vec![AudioTrack {
                    codec: "opus".into(),
                    ..track(0, 2, "")
                }],
            ),
            20_905_984,
            10_000_000,
            &discord,
            Decision::Encode,
        ),
        (
            "no audio, mp4 h264: keep",
            src("mp4", "h264", vec![]),
            20_905_984,
            10_000_000,
            &discord,
            Decision::KeepOriginal,
        ),
        (
            "two audio tracks always encode (the mix needs it)",
            src("mkv", "h264", vec![track(0, 2, "Game"), track(1, 2, "Mic")]),
            20_905_984,
            10_000_000,
            &discord,
            Decision::Encode,
        ),
        (
            "MP4 upper-case tokens match",
            src("MP4", "H264", one_aac),
            20_905_984,
            10_000_000,
            &discord,
            Decision::KeepOriginal,
        ),
    ];
    for (name, probe, hard, size, allowed, expect) in table {
        assert_eq!(
            keep_original_or_remux(&probe, hard, size, allowed),
            expect,
            "{name}"
        );
    }
}

// ---------------------------------------------------------------- 3.5.6 encoder chain

#[test]
fn encoder_chain_per_platform() {
    use Platform::*;
    let table: &[(&str, Platform, Option<Gpu>, bool, bool, bool, &[&str])] = &[
        (
            "windows nvidia",
            Windows,
            Some(Gpu::Nvidia),
            false,
            true,
            true,
            &["h264_nvenc", "h264_mf", "libvpx-vp9"],
        ),
        (
            "windows amd, no webm",
            Windows,
            Some(Gpu::Amd),
            false,
            true,
            false,
            &["h264_amf", "h264_mf"],
        ),
        (
            "windows intel",
            Windows,
            Some(Gpu::Intel),
            false,
            true,
            true,
            &["h264_qsv", "h264_mf", "libvpx-vp9"],
        ),
        (
            "windows no gpu",
            Windows,
            None,
            false,
            true,
            true,
            &["h264_mf", "libvpx-vp9"],
        ),
        (
            "windows faster off keeps media foundation",
            Windows,
            Some(Gpu::Nvidia),
            false,
            false,
            true,
            &["h264_mf", "libvpx-vp9"],
        ),
        (
            "windows user ffmpeg with x264 goes first",
            Windows,
            Some(Gpu::Nvidia),
            true,
            true,
            true,
            &["libx264", "h264_nvenc", "h264_mf", "libvpx-vp9"],
        ),
        (
            "macos",
            MacOs,
            None,
            false,
            true,
            true,
            &["h264_videotoolbox", "libvpx-vp9"],
        ),
        (
            "macos faster off keeps videotoolbox",
            MacOs,
            None,
            false,
            false,
            false,
            &["h264_videotoolbox"],
        ),
        (
            "macos with x264",
            MacOs,
            None,
            true,
            true,
            false,
            &["libx264", "h264_videotoolbox"],
        ),
        (
            "linux nvidia",
            Linux,
            Some(Gpu::Nvidia),
            false,
            true,
            true,
            &["h264_nvenc", "h264_vaapi", "libvpx-vp9"],
        ),
        (
            "linux intel",
            Linux,
            Some(Gpu::Intel),
            false,
            true,
            true,
            &["h264_qsv", "h264_vaapi", "libvpx-vp9"],
        ),
        (
            "linux amd",
            Linux,
            Some(Gpu::Amd),
            false,
            true,
            false,
            &["h264_amf", "h264_vaapi"],
        ),
        (
            "linux no gpu",
            Linux,
            None,
            false,
            true,
            true,
            &["h264_vaapi", "libvpx-vp9"],
        ),
        (
            "linux faster off, no webm: nothing (NoEncoder)",
            Linux,
            None,
            false,
            false,
            false,
            &[],
        ),
        (
            "linux faster off, webm allowed",
            Linux,
            Some(Gpu::Nvidia),
            false,
            false,
            true,
            &["libvpx-vp9"],
        ),
    ];
    for &(name, platform, gpu, x264, faster, webm, expect) in table {
        assert_eq!(
            encoder_chain(platform, gpu, x264, faster, webm),
            expect,
            "{name}"
        );
    }
}

// ---------------------------------------------------------------- serialisation shape

#[test]
fn plan_serialises_snake_case_like_cia_core() {
    let p = plan(
        &obs_clip(70_000),
        &DISCORD_FREE,
        &h264_mp4(discord_caps()),
        EncoderKind::WebCodecs,
        &PlanOptions::default(),
    )
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(v["container"], "mp4");
    assert_eq!(v["video_codec"], "h264");
    assert_eq!(v["quality"], "good");
    assert_eq!(v["predicted_bytes"], 19_238_810);
    let d = serde_json::to_value(Decision::Remux {
        container: Container::Webm,
    })
    .unwrap();
    assert_eq!(
        d,
        serde_json::json!({ "type": "remux", "container": "webm" })
    );
    let e = serde_json::to_value(EncoderKind::SinglePassHardware).unwrap();
    assert_eq!(e, "single_pass_hardware");
    let o: PlanOptions =
        serde_json::from_str(r#"{"audio":{"type":"track","index":1},"trim":[0,5000]}"#).unwrap();
    assert_eq!(o.audio, AudioTrackChoice::Track { index: 1 });
    assert_eq!(o.trim, Some((0, 5000)));
}
