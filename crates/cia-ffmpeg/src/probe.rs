//! `ffprobe` into a [`VideoProbe`] (DESIGN.md 3.5.1).
//!
//! Two calls: format plus every stream as JSON, and the first five seconds
//! of video frames for the frame-rate regularity check. A failure of the
//! first call is the "damaged or incomplete" error; a failure of the second
//! only disables the VFR detection.

use crate::command::{probe_args, probe_frames_args};
use crate::process::run_capture;
use crate::FfmpegError;
use cia_video_plan::{AudioTrack, VideoProbe};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// ffprobe should answer well within this on any file it can read.
pub const PROBE_DEADLINE: Duration = Duration::from_secs(60);

/// Probe `input` with the given `ffprobe` binary.
pub fn probe(ffprobe: &Path, input: &Path) -> Result<VideoProbe, FfmpegError> {
    let main = run_capture(ffprobe, probe_args(input), PROBE_DEADLINE, None)?;
    if !main.success() {
        return Err(FfmpegError::DamagedInput {
            detail: main.stderr_str().trim().to_string(),
        });
    }
    let frames = run_capture(ffprobe, probe_frames_args(input), PROBE_DEADLINE, None)
        .ok()
        .filter(|c| c.success())
        .map(|c| c.stdout_str());
    let file_name = input.file_name().map(|n| n.to_string_lossy().into_owned());
    parse_probe(&main.stdout_str(), frames.as_deref(), file_name.as_deref())
}

/// Build the probe from ffprobe's JSON. `file_name` disambiguates containers
/// ffprobe groups together ("mov,mp4,m4a,3gp,3g2,mj2", "matroska,webm").
pub fn parse_probe(
    format_json: &str,
    frames_json: Option<&str>,
    file_name: Option<&str>,
) -> Result<VideoProbe, FfmpegError> {
    let doc: Value = serde_json::from_str(format_json).map_err(|e| FfmpegError::DamagedInput {
        detail: format!("ffprobe output is not JSON: {e}"),
    })?;
    let streams = doc["streams"].as_array().cloned().unwrap_or_default();
    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .ok_or_else(|| FfmpegError::DamagedInput {
            detail: "no video stream".to_string(),
        })?;

    let format = &doc["format"];
    let duration_ms = seconds_to_ms(&format["duration"])
        .or_else(|| seconds_to_ms(&video["duration"]))
        .unwrap_or(0);
    let container = container_token(str_of(&format["format_name"]), file_name);

    let coded_w = u32_of(&video["width"]);
    let coded_h = u32_of(&video["height"]);
    let rotation = rotation_degrees(video);
    let (display_w, display_h) = if rotation % 180 != 0 {
        (coded_h, coded_w)
    } else {
        (coded_w, coded_h)
    };
    let avg_fps = parse_ratio(str_of(&video["avg_frame_rate"]));
    let r_fps = parse_ratio(str_of(&video["r_frame_rate"]));
    let avg_fps = if avg_fps > 0.0 { avg_fps } else { r_fps };
    let max_fps = r_fps.max(avg_fps);

    let color_transfer = str_of(&video["color_transfer"]).to_string();
    let color_primaries = str_of(&video["color_primaries"]).to_string();
    let is_hdr = matches!(color_transfer.as_str(), "smpte2084" | "arib-std-b67");
    let dolby_vision_profile = dovi_profile(video);

    let audio = streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .enumerate()
        .map(|(i, s)| AudioTrack {
            index: i as u32,
            codec: str_of(&s["codec_name"]).to_ascii_lowercase(),
            channels: u32_of(&s["channels"]),
            sample_rate: u32_of(&s["sample_rate"]),
            bitrate_bps: u64_of(&s["bit_rate"]).filter(|b| *b > 0),
            title: s["tags"]["title"]
                .as_str()
                .or_else(|| s["tags"]["TITLE"].as_str())
                .map(str::to_string),
        })
        .collect();

    let is_vfr = frames_json.is_some_and(is_vfr_from_frames);

    Ok(VideoProbe {
        duration_ms,
        container,
        video_codec: str_of(&video["codec_name"]).to_ascii_lowercase(),
        coded_w,
        coded_h,
        display_w,
        display_h,
        rotation_degrees: rotation,
        avg_fps,
        max_fps,
        is_vfr,
        pixel_format: str_of(&video["pix_fmt"]).to_string(),
        color_transfer,
        color_primaries,
        is_hdr,
        dolby_vision_profile,
        audio,
    })
}

/// Rotation from the display matrix side data, else the legacy `rotate` tag,
/// normalised to 0, 90, 180 or 270.
fn rotation_degrees(video: &Value) -> i32 {
    let from_matrix = video["side_data_list"]
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|sd| sd["side_data_type"] == "Display Matrix")
                .and_then(|sd| sd["rotation"].as_f64())
        })
        .map(|r| r.round() as i32);
    let from_tag = video["tags"]["rotate"]
        .as_str()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|r| r.round() as i32);
    let raw = from_matrix.or(from_tag).unwrap_or(0);
    // ffprobe reports counter-clockwise as negative; the sign does not change
    // the display size, only the quadrant.
    let normalised = raw.rem_euclid(360);
    // Snap to the nearest right angle; anything else is not a phone rotation.
    ((normalised + 45) / 90 * 90) % 360
}

fn dovi_profile(video: &Value) -> Option<u8> {
    video["side_data_list"].as_array().and_then(|list| {
        list.iter()
            .find(|sd| {
                sd["side_data_type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("DOVI"))
            })
            .and_then(|sd| sd["dv_profile"].as_u64())
            .map(|p| p as u8)
    })
}

/// Lower-case container token from `format_name` and the file extension.
pub fn container_token(format_name: &str, file_name: Option<&str>) -> String {
    let ext = file_name.and_then(|n| {
        Path::new(n)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
    });
    let names: Vec<&str> = format_name.split(',').map(str::trim).collect();
    if names.contains(&"mp4") || names.contains(&"mov") {
        return match ext.as_deref() {
            Some(e @ ("mov" | "m4v" | "3gp" | "mp4")) => e.to_string(),
            _ => "mp4".to_string(),
        };
    }
    if names.contains(&"matroska") || names.contains(&"webm") {
        return match ext.as_deref() {
            Some("webm") => "webm".to_string(),
            _ => "mkv".to_string(),
        };
    }
    names
        .first()
        .map(|n| n.to_ascii_lowercase())
        .unwrap_or_default()
}

/// More than 2 percent spread in the frame timing over the sampled frames.
///
/// Three signals, any of which marks the clip VFR: the packet durations
/// vary, the gaps between consecutive timestamps vary, or the declared
/// packet duration disagrees with the actual spacing (what a dropped-frame
/// phone recording looks like once muxed). Fewer than three frames: not VFR.
pub fn is_vfr_from_frames(frames_json: &str) -> bool {
    let Ok(doc) = serde_json::from_str::<Value>(frames_json) else {
        return false;
    };
    let Some(frames) = doc["frames"].as_array() else {
        return false;
    };
    let durations: Vec<f64> = frames
        .iter()
        .filter_map(|f| {
            f64_of(&f["pkt_duration_time"])
                .or_else(|| f64_of(&f["duration_time"]))
                .filter(|d| *d > 0.0)
        })
        .collect();
    let mut stamps: Vec<f64> = frames
        .iter()
        .filter_map(|f| {
            f64_of(&f["best_effort_timestamp_time"])
                .or_else(|| f64_of(&f["pts_time"]))
                .or_else(|| f64_of(&f["pkt_pts_time"]))
                .or_else(|| f64_of(&f["pkt_dts_time"]))
        })
        .collect();
    stamps.sort_by(|a, b| a.total_cmp(b));
    let intervals: Vec<f64> = stamps
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.0)
        .collect();
    if frames.len() < 3 {
        return false;
    }
    if spread(&durations) > 0.02 || spread(&intervals) > 0.02 {
        return true;
    }
    match (median(&durations), median(&intervals)) {
        (Some(d), Some(i)) if d > 0.0 => ((i - d) / d).abs() > 0.02,
        _ => false,
    }
}

fn spread(xs: &[f64]) -> f64 {
    let Some(med) = median(xs) else {
        return 0.0;
    };
    if med <= 0.0 {
        return 0.0;
    }
    let max = xs.iter().cloned().fold(f64::MIN, f64::max);
    let min = xs.iter().cloned().fold(f64::MAX, f64::min);
    (max - min) / med
}

fn median(xs: &[f64]) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

/// "30000/1001" -> 29.97; "0/0" -> 0.
pub fn parse_ratio(s: &str) -> f32 {
    match s.split_once('/') {
        Some((n, d)) => {
            let n: f32 = n.trim().parse().unwrap_or(0.0);
            let d: f32 = d.trim().parse().unwrap_or(0.0);
            if d > 0.0 {
                n / d
            } else {
                0.0
            }
        }
        None => s.trim().parse().unwrap_or(0.0),
    }
}

fn str_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn u32_of(v: &Value) -> u32 {
    match v {
        Value::Number(n) => n.as_u64().unwrap_or(0) as u32,
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn u64_of(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn f64_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn seconds_to_ms(v: &Value) -> Option<u64> {
    f64_of(v)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROTATED: &str = r#"{
      "streams": [
        {"index":0,"codec_name":"h264","codec_type":"video","width":1280,"height":720,
         "pix_fmt":"yuv420p","r_frame_rate":"30/1","avg_frame_rate":"30/1","duration":"10.000000",
         "side_data_list":[{"side_data_type":"Display Matrix","displaymatrix":"...","rotation":-90}]},
        {"index":1,"codec_name":"aac","codec_type":"audio","sample_rate":"48000","channels":1,
         "bit_rate":"127393","tags":{"title":"Mic"}}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10.000000","size":"5605361"}
    }"#;

    #[test]
    fn rotation_swaps_display_dimensions() {
        let p = parse_probe(ROTATED, None, Some("clip.MOV")).unwrap();
        assert_eq!((p.coded_w, p.coded_h), (1280, 720));
        assert_eq!((p.display_w, p.display_h), (720, 1280));
        assert_eq!(p.rotation_degrees, 270);
        assert!(p.is_portrait());
        assert_eq!(p.container, "mov");
        assert_eq!(p.duration_ms, 10_000);
        assert_eq!(p.audio.len(), 1);
        assert_eq!(p.audio[0].title.as_deref(), Some("Mic"));
        assert_eq!(p.audio[0].bitrate_bps, Some(127_393));
        assert_eq!(p.audio[0].channels, 1);
        assert!(!p.is_hdr);
    }

    #[test]
    fn rotate_tag_fallback_and_hdr_and_dovi() {
        let json = r#"{"streams":[{"codec_type":"video","codec_name":"hevc","width":3840,"height":2160,
          "pix_fmt":"yuv420p10le","color_transfer":"arib-std-b67","color_primaries":"bt2020",
          "r_frame_rate":"60000/1001","avg_frame_rate":"30000/1001",
          "tags":{"rotate":"90"},
          "side_data_list":[{"side_data_type":"DOVI configuration record","dv_profile":8,"dv_level":5}]}],
          "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"3.5"}}"#;
        let p = parse_probe(json, None, Some("IMG_0001.mov")).unwrap();
        assert_eq!(p.rotation_degrees, 90);
        assert_eq!((p.display_w, p.display_h), (2160, 3840));
        assert!(p.is_hdr);
        assert_eq!(p.dolby_vision_profile, Some(8));
        assert!((p.avg_fps - 29.97).abs() < 0.01);
        assert!((p.max_fps - 59.94).abs() < 0.01);
        assert_eq!(p.duration_ms, 3500);
        assert_eq!(p.video_codec, "hevc");
        assert!(p.audio.is_empty());
    }

    #[test]
    fn no_video_stream_is_damaged() {
        let json = r#"{"streams":[{"codec_type":"audio","codec_name":"mp3"}],"format":{"format_name":"mp3"}}"#;
        let err = parse_probe(json, None, None).unwrap_err();
        assert_eq!(err.failure_code(), "damaged_video");
        assert_eq!(
            parse_probe("{", None, None).unwrap_err().failure_code(),
            "damaged_video"
        );
    }

    #[test]
    fn container_tokens() {
        assert_eq!(
            container_token("mov,mp4,m4a,3gp,3g2,mj2", Some("a.mp4")),
            "mp4"
        );
        assert_eq!(
            container_token("mov,mp4,m4a,3gp,3g2,mj2", Some("a.MOV")),
            "mov"
        );
        assert_eq!(container_token("mov,mp4,m4a,3gp,3g2,mj2", None), "mp4");
        assert_eq!(container_token("matroska,webm", Some("a.mkv")), "mkv");
        assert_eq!(container_token("matroska,webm", Some("a.webm")), "webm");
        assert_eq!(container_token("avi", Some("a.avi")), "avi");
        assert_eq!(container_token("asf", Some("a.wmv")), "asf");
    }

    fn frames(durations: &[f64], stamps: &[f64]) -> String {
        let items: Vec<String> = durations
            .iter()
            .zip(stamps)
            .map(|(d, t)| {
                format!(r#"{{"pkt_duration_time":"{d:.6}","best_effort_timestamp_time":"{t:.6}"}}"#)
            })
            .collect();
        format!(r#"{{"frames":[{}]}}"#, items.join(","))
    }

    #[test]
    fn vfr_detection() {
        let cfr = frames(&[0.0333; 6], &[0.0, 0.0333, 0.0667, 0.1, 0.1333, 0.1667]);
        assert!(!is_vfr_from_frames(&cfr));
        let vary = frames(
            &[0.0333, 0.0333, 0.05, 0.0333, 0.0333, 0.0333],
            &[0.0, 0.0333, 0.0667, 0.1167, 0.15, 0.1833],
        );
        assert!(is_vfr_from_frames(&vary));
        // declared 1/60 s per packet but spaced 7/60 s apart (the synthetic fixture)
        let mismatch = frames(&[0.016667; 5], &[0.5, 0.616667, 0.733333, 0.85, 0.966667]);
        assert!(is_vfr_from_frames(&mismatch));
        // 6.x field name
        let six = r#"{"frames":[{"duration_time":"0.033333","pts_time":"0"},{"duration_time":"0.033333","pts_time":"0.033333"},{"duration_time":"0.033333","pts_time":"0.066667"}]}"#;
        assert!(!is_vfr_from_frames(six));
        assert!(!is_vfr_from_frames("not json"));
        assert!(!is_vfr_from_frames(r#"{"frames":[]}"#));
    }

    #[test]
    fn ratios() {
        assert_eq!(parse_ratio("30/1"), 30.0);
        assert!((parse_ratio("30000/1001") - 29.97).abs() < 0.001);
        assert_eq!(parse_ratio("0/0"), 0.0);
        assert_eq!(parse_ratio("25"), 25.0);
        assert_eq!(parse_ratio(""), 0.0);
    }
}
