//! Argument builders for every FFmpeg command Smidge runs (DESIGN.md 3.5.7).
//!
//! Pure functions: `Vec<OsString>` in, nothing executed. Paths go in as
//! `OsString`s so the runner hands them to `Command::arg` unchanged.
//!
//! One deviation from the design's sketch: with `-filter_complex` the
//! outputs of the graph are mapped by label (`-map [v] -map [a]`) instead of
//! `-map 0:v:0 -map 0:a`, which would include the raw streams a second time.
//! The graph inputs `[0:v:0]` and `[0:a:N]` carry the stream selection.

use cia_video_plan::{AudioCodec, Container, EncoderKind, VideoCodec};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The video encoders Smidge knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoder {
    /// Only from a user-supplied FFmpeg (LGPL build has no x264). Two-pass.
    Libx264,
    /// VP9 WebM fallback. Two-pass.
    LibvpxVp9,
    H264Videotoolbox,
    H264Nvenc,
    H264Amf,
    H264Qsv,
    H264Vaapi,
    /// Media Foundation; `hw_encoding` false is the retry after a hardware failure.
    H264Mf {
        hw_encoding: bool,
    },
}

impl Encoder {
    /// Every encoder in chain order (3.5.6), for detection and tests.
    pub const ALL: [Encoder; 8] = [
        Encoder::Libx264,
        Encoder::H264Videotoolbox,
        Encoder::H264Nvenc,
        Encoder::H264Amf,
        Encoder::H264Qsv,
        Encoder::H264Vaapi,
        Encoder::H264Mf { hw_encoding: true },
        Encoder::LibvpxVp9,
    ];

    /// FFmpeg's encoder name, as `-encoders` lists it and `encoder_chain` returns it.
    pub fn parse(name: &str) -> Option<Encoder> {
        Some(match name {
            "libx264" => Encoder::Libx264,
            "libvpx-vp9" => Encoder::LibvpxVp9,
            "h264_videotoolbox" => Encoder::H264Videotoolbox,
            "h264_nvenc" => Encoder::H264Nvenc,
            "h264_amf" => Encoder::H264Amf,
            "h264_qsv" => Encoder::H264Qsv,
            "h264_vaapi" => Encoder::H264Vaapi,
            "h264_mf" => Encoder::H264Mf { hw_encoding: true },
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Encoder::Libx264 => "libx264",
            Encoder::LibvpxVp9 => "libvpx-vp9",
            Encoder::H264Videotoolbox => "h264_videotoolbox",
            Encoder::H264Nvenc => "h264_nvenc",
            Encoder::H264Amf => "h264_amf",
            Encoder::H264Qsv => "h264_qsv",
            Encoder::H264Vaapi => "h264_vaapi",
            Encoder::H264Mf { .. } => "h264_mf",
        }
    }

    /// Label for the attempt log: "libvpx-vp9-2pass", "h264_nvenc", "h264_mf-sw".
    pub fn attempt_label(self) -> String {
        match self {
            Encoder::Libx264 | Encoder::LibvpxVp9 => format!("{}-2pass", self.name()),
            Encoder::H264Mf { hw_encoding: false } => "h264_mf-sw".to_string(),
            other => other.name().to_string(),
        }
    }

    pub fn kind(self) -> EncoderKind {
        match self {
            Encoder::Libx264 | Encoder::LibvpxVp9 => EncoderKind::TwoPassSoftware,
            _ => EncoderKind::SinglePassHardware,
        }
    }

    pub fn two_pass(self) -> bool {
        self.kind().two_pass()
    }

    pub fn is_hardware(self) -> bool {
        !matches!(self, Encoder::Libx264 | Encoder::LibvpxVp9)
    }

    pub fn video_codec(self) -> VideoCodec {
        match self {
            Encoder::LibvpxVp9 => VideoCodec::Vp9,
            _ => VideoCodec::H264,
        }
    }

    /// The container this encoder's output goes into.
    pub fn container(self) -> Container {
        match self {
            Encoder::LibvpxVp9 => Container::Webm,
            _ => Container::Mp4,
        }
    }

    /// Rate-control arguments from the 3.5.7 table, without the pass flags.
    fn rate_control(self, bps: u64) -> Vec<OsString> {
        let br = bps.to_string();
        let scaled = |f: f64| ((bps as f64) * f).round().to_string();
        let mut v: Vec<String> = Vec::new();
        macro_rules! a {
            ($($x:expr),* $(,)?) => {{ $( v.push($x.to_string()); )* }};
        }
        match self {
            Encoder::LibvpxVp9 => a!(
                "-c:v",
                "libvpx-vp9",
                "-b:v",
                br,
                "-minrate",
                scaled(0.5),
                "-maxrate",
                scaled(1.45),
                "-row-mt",
                "1",
                "-tile-columns",
                "2",
                "-deadline",
                "good"
            ),
            Encoder::Libx264 => a!(
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-profile:v",
                "high",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.5),
                "-bufsize",
                scaled(2.0)
            ),
            Encoder::H264Videotoolbox => a!(
                "-c:v",
                "h264_videotoolbox",
                "-profile:v",
                "high",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.2),
                "-bufsize",
                scaled(2.0),
                "-allow_sw",
                "1",
                "-realtime",
                "0"
            ),
            Encoder::H264Nvenc => a!(
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p6",
                "-tune",
                "hq",
                "-multipass",
                "fullres",
                "-rc",
                "vbr",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.3),
                "-bufsize",
                scaled(2.0),
                "-profile:v",
                "high"
            ),
            Encoder::H264Amf => a!(
                "-c:v",
                "h264_amf",
                "-quality",
                "quality",
                "-rc",
                "vbr_peak",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.3),
                "-bufsize",
                scaled(2.0),
                "-profile:v",
                "high"
            ),
            Encoder::H264Qsv => a!(
                "-c:v",
                "h264_qsv",
                "-preset",
                "slower",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.3),
                "-bufsize",
                scaled(2.0),
                "-profile:v",
                "high"
            ),
            Encoder::H264Vaapi => a!(
                "-c:v",
                "h264_vaapi",
                "-rc_mode",
                "VBR",
                "-b:v",
                br,
                "-maxrate",
                scaled(1.3)
            ),
            Encoder::H264Mf { hw_encoding } => a!(
                "-c:v",
                "h264_mf",
                "-rate_control",
                "pc_vbr",
                "-b:v",
                br,
                "-scenario",
                "display_remoting",
                "-hw_encoding",
                if hw_encoding { "1" } else { "0" }
            ),
        }
        v.into_iter().map(OsString::from).collect()
    }
}

/// Which pass of a two-pass encode, or a single pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass<'a> {
    Single,
    /// First pass: no audio, `-f null -`, stats written to `passlog`.
    First {
        passlog: &'a Path,
    },
    /// Second pass: real output, stats read from `passlog`.
    Second {
        passlog: &'a Path,
    },
}

/// The output audio stream.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioOut {
    pub codec: AudioCodec,
    pub bps: u64,
    pub channels: u32,
    /// Source audio stream indices (`0:a:N`). More than one means `amix`.
    pub tracks: Vec<u32>,
}

/// One encode (one pass) of one video, as the planner decided it.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeJob<'a> {
    pub input: &'a Path,
    /// Where this pass writes; the caller passes the `.partial` name.
    pub output: &'a Path,
    /// `(start_ms, end_ms)` in source time.
    pub trim: Option<(u64, u64)>,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub video_bps: u64,
    /// PQ or HLG source: tone-map to BT.709.
    pub hdr: bool,
    pub encoder: Encoder,
    pub container: Container,
    /// None: no audio stream in the output.
    pub audio: Option<AudioOut>,
    pub pass: Pass<'a>,
}

/// Common prefix of every ffmpeg command Smidge runs.
pub const COMMON_PREFIX: [&str; 8] = [
    "-hide_banner",
    "-nostdin",
    "-y",
    "-progress",
    "pipe:1",
    "-nostats",
    "-v",
    "error",
];

/// `name.ext` -> `name.partial.ext`; `name` -> `name.partial`.
pub fn partial_path(final_path: &Path) -> PathBuf {
    let stem = final_path
        .file_stem()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    let mut name = stem;
    name.push(".partial");
    if let Some(ext) = final_path.extension() {
        name.push(".");
        name.push(ext);
    }
    final_path.with_file_name(name)
}

/// Seconds with millisecond precision, "2.500".
pub fn seconds(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// "30" for whole frame rates, "29.97" otherwise.
pub fn fps_arg(fps: f32) -> String {
    if (fps - fps.round()).abs() < 1e-4 {
        format!("{}", fps.round() as u32)
    } else {
        let s = format!("{fps:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The video filter chain of 3.5.7 for the given size and dynamic range.
pub fn video_filter(width: u32, height: u32, hdr: bool, encoder: Encoder) -> String {
    let scale = format!("scale={width}:{height}:flags=lanczos,setsar=1");
    let mut chain = if hdr {
        format!(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
             tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p,{scale}"
        )
    } else {
        scale
    };
    if encoder == Encoder::H264Vaapi {
        chain.push_str(",format=nv12,hwupload");
    }
    chain
}

/// The audio part of the filter graph: `aresample` per track, then `amix`
/// and `alimiter` when more than one track is selected. Ends in `[a]`.
pub fn audio_filter(tracks: &[u32]) -> String {
    const RESAMPLE: &str = "aresample=async=1:first_pts=0";
    match tracks {
        [] => String::new(),
        [one] => format!("[0:a:{one}]{RESAMPLE}[a]"),
        many => {
            let mut parts: Vec<String> = many
                .iter()
                .enumerate()
                .map(|(i, t)| format!("[0:a:{t}]{RESAMPLE}[a{i}]"))
                .collect();
            let inputs: String = (0..many.len()).map(|i| format!("[a{i}]")).collect();
            parts.push(format!(
                "{inputs}amix=inputs={}:duration=longest:normalize=0,alimiter=limit=0.97[a]",
                many.len()
            ));
            parts.join(";")
        }
    }
}

/// The full argument vector for one pass of an encode (3.5.7).
pub fn encode_args(job: &EncodeJob<'_>) -> Vec<OsString> {
    let mut args: Vec<OsString> = COMMON_PREFIX.iter().map(OsString::from).collect();
    if job.encoder == Encoder::H264Vaapi {
        push(&mut args, &["-vaapi_device", "/dev/dri/renderD128"]);
    }
    args.push("-i".into());
    args.push(job.input.as_os_str().to_os_string());
    if let Some((start, end)) = job.trim {
        push(&mut args, &["-ss", &seconds(start), "-to", &seconds(end)]);
    }

    let first_pass = matches!(job.pass, Pass::First { .. });
    let audio = if first_pass {
        None
    } else {
        job.audio.as_ref().filter(|a| !a.tracks.is_empty())
    };

    let mut graph = format!(
        "[0:v:0]{}[v]",
        video_filter(job.width, job.height, job.hdr, job.encoder)
    );
    if let Some(a) = audio {
        graph.push(';');
        graph.push_str(&audio_filter(&a.tracks));
    }
    push(&mut args, &["-filter_complex", &graph, "-map", "[v]"]);
    if audio.is_some() {
        push(&mut args, &["-map", "[a]"]);
    }

    args.extend(job.encoder.rate_control(job.video_bps));
    match job.pass {
        Pass::Single => {}
        Pass::First { passlog } | Pass::Second { passlog } => {
            if job.encoder == Encoder::LibvpxVp9 {
                let cpu_used = if first_pass { "4" } else { "2" };
                push(&mut args, &["-cpu-used", cpu_used]);
            }
            push(&mut args, &["-pass", if first_pass { "1" } else { "2" }]);
            args.push("-passlogfile".into());
            args.push(passlog.as_os_str().to_os_string());
        }
    }

    push(&mut args, &["-fps_mode", "cfr", "-r", &fps_arg(job.fps)]);
    if job.encoder != Encoder::H264Vaapi {
        push(&mut args, &["-pix_fmt", "yuv420p"]);
    }
    if job.hdr {
        push(
            &mut args,
            &[
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
            ],
        );
    }

    if first_pass {
        push(
            &mut args,
            &["-an", "-map_metadata", "-1", "-map_chapters", "-1"],
        );
        push(&mut args, &["-f", "null", "-"]);
        return args;
    }

    if let Some(a) = audio {
        let codec = match a.codec {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "libopus",
        };
        push(
            &mut args,
            &[
                "-c:a",
                codec,
                "-b:a",
                &a.bps.to_string(),
                "-ac",
                &a.channels.to_string(),
            ],
        );
    }
    push(&mut args, &["-map_metadata", "-1", "-map_chapters", "-1"]);
    if job.container == Container::Mp4 {
        push(&mut args, &["-movflags", "+faststart"]);
    }
    args.push(job.output.as_os_str().to_os_string());
    args
}

/// `-c copy` remux for a source whose streams already fit (3.5.4).
pub fn remux_args(
    input: &Path,
    output: &Path,
    container: Container,
    audio_track: Option<u32>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = COMMON_PREFIX.iter().map(OsString::from).collect();
    args.push("-i".into());
    args.push(input.as_os_str().to_os_string());
    push(&mut args, &["-map", "0:v:0"]);
    if let Some(t) = audio_track {
        push(&mut args, &["-map", &format!("0:a:{t}")]);
    }
    push(
        &mut args,
        &["-c", "copy", "-map_metadata", "-1", "-map_chapters", "-1"],
    );
    if container == Container::Mp4 {
        push(&mut args, &["-movflags", "+faststart"]);
    }
    args.push(output.as_os_str().to_os_string());
    args
}

/// The one-second test encode that proves an encoder works on this machine (3.5.6).
pub fn test_encode_args(encoder: Encoder) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error", "-nostats"]
        .iter()
        .map(OsString::from)
        .collect();
    if encoder == Encoder::H264Vaapi {
        push(&mut args, &["-vaapi_device", "/dev/dri/renderD128"]);
    }
    push(
        &mut args,
        &["-f", "lavfi", "-i", "testsrc2=s=640x360:r=30", "-t", "1"],
    );
    if encoder == Encoder::H264Vaapi {
        push(&mut args, &["-vf", "format=nv12,hwupload"]);
    } else {
        push(&mut args, &["-pix_fmt", "yuv420p"]);
    }
    push(&mut args, &["-c:v", encoder.name()]);
    if let Encoder::H264Mf { hw_encoding } = encoder {
        push(
            &mut args,
            &["-hw_encoding", if hw_encoding { "1" } else { "0" }],
        );
    }
    push(&mut args, &["-f", "null", "-"]);
    args
}

/// One-second test encode for an audio encoder ("aac", "libopus", "libmp3lame").
pub fn test_audio_encode_args(encoder: &str) -> Vec<OsString> {
    [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-nostats",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "1",
        "-c:a",
        encoder,
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(OsString::from)
    .collect()
}

pub fn version_args() -> Vec<OsString> {
    vec!["-hide_banner".into(), "-version".into()]
}

pub fn encoders_list_args() -> Vec<OsString> {
    vec!["-hide_banner".into(), "-encoders".into()]
}

/// `ffprobe` for format and all streams (3.5.1).
pub fn probe_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(input.as_os_str().to_os_string());
    args
}

/// `ffprobe` for the first five seconds of video frames, for `is_vfr` (3.5.1).
pub fn probe_frames_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_frames",
        "-read_intervals",
        "%+5",
        "-select_streams",
        "v:0",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(input.as_os_str().to_os_string());
    args
}

/// Which end of the file a decode check covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    First,
    Last,
}

/// Decode the first or last second of `input` to nothing (3.11 video row).
pub fn decode_check_args(input: &Path, edge: Edge) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error", "-nostats"]
        .iter()
        .map(OsString::from)
        .collect();
    if edge == Edge::Last {
        push(&mut args, &["-sseof", "-1"]);
    }
    args.push("-i".into());
    args.push(input.as_os_str().to_os_string());
    if edge == Edge::First {
        push(&mut args, &["-t", "1"]);
    }
    push(&mut args, &["-f", "null", "-"]);
    args
}

/// One JPEG frame at `ms`, at most 1280 px wide, to stdout (trim preview).
pub fn extract_frame_args(input: &Path, ms: u64) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error", "-nostats"]
        .iter()
        .map(OsString::from)
        .collect();
    push(&mut args, &["-ss", &seconds(ms)]);
    args.push("-i".into());
    args.push(input.as_os_str().to_os_string());
    push(
        &mut args,
        &[
            "-frames:v",
            "1",
            "-vf",
            "scale='min(1280,iw)':-2",
            "-f",
            "image2pipe",
            "-c:v",
            "mjpeg",
            "-q:v",
            "3",
            "-",
        ],
    );
    args
}

/// First frame of a still image (HEIC/AVIF) as RGBA PNG to stdout.
pub fn decode_image_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error", "-nostats"]
        .iter()
        .map(OsString::from)
        .collect();
    args.push("-i".into());
    args.push(input.as_os_str().to_os_string());
    push(
        &mut args,
        &[
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgba",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "-",
        ],
    );
    args
}

/// Render an argument vector as one line for logs and snapshots.
pub fn render(args: &[OsString]) -> String {
    args.iter()
        .map(|a| {
            let s = a.to_string_lossy();
            if s.is_empty() || s.contains(|c: char| c.is_whitespace() || c == ';') {
                format!("\"{s}\"")
            } else {
                s.into_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job<'a>(encoder: Encoder, pass: Pass<'a>) -> EncodeJob<'a> {
        EncodeJob {
            input: Path::new("clip.mp4"),
            output: Path::new(if encoder == Encoder::LibvpxVp9 {
                "clip.partial.webm"
            } else {
                "clip.partial.mp4"
            }),
            trim: None,
            width: 1280,
            height: 720,
            fps: 30.0,
            video_bps: 2_000_000,
            hdr: false,
            encoder,
            container: encoder.container(),
            audio: Some(AudioOut {
                codec: if encoder == Encoder::LibvpxVp9 {
                    AudioCodec::Opus
                } else {
                    AudioCodec::Aac
                },
                bps: 128_000,
                channels: 2,
                tracks: vec![0],
            }),
            pass,
        }
    }

    #[test]
    fn snapshot_every_encoder() {
        let passlog = Path::new("passlog");
        for encoder in Encoder::ALL {
            let name = encoder.name();
            if encoder.two_pass() {
                let first = encode_args(&job(encoder, Pass::First { passlog }));
                let second = encode_args(&job(encoder, Pass::Second { passlog }));
                insta::assert_snapshot!(format!("{name}_pass1"), render(&first));
                insta::assert_snapshot!(format!("{name}_pass2"), render(&second));
            } else {
                let single = encode_args(&job(encoder, Pass::Single));
                insta::assert_snapshot!(name, render(&single));
            }
        }
        let sw = encode_args(&job(Encoder::H264Mf { hw_encoding: false }, Pass::Single));
        insta::assert_snapshot!("h264_mf_software", render(&sw));
    }

    #[test]
    fn snapshot_hdr_trim_and_mix() {
        let mut j = job(Encoder::H264Nvenc, Pass::Single);
        j.hdr = true;
        j.trim = Some((2_500, 7_000));
        j.width = 608;
        j.height = 1080;
        j.fps = 29.97;
        j.audio = Some(AudioOut {
            codec: AudioCodec::Aac,
            bps: 96_000,
            channels: 2,
            tracks: vec![0, 1],
        });
        insta::assert_snapshot!("hdr_trim_mix_nvenc", render(&encode_args(&j)));
    }

    #[test]
    fn snapshot_no_audio_and_single_track_choice() {
        let mut j = job(Encoder::H264Videotoolbox, Pass::Single);
        j.audio = None;
        insta::assert_snapshot!("no_audio_videotoolbox", render(&encode_args(&j)));
        let mut j = job(Encoder::H264Qsv, Pass::Single);
        j.audio.as_mut().unwrap().tracks = vec![1];
        insta::assert_snapshot!("track1_qsv", render(&encode_args(&j)));
    }

    #[test]
    fn snapshot_helpers() {
        insta::assert_snapshot!(
            "remux_mp4",
            render(&remux_args(
                Path::new("in.mkv"),
                Path::new("out.partial.mp4"),
                Container::Mp4,
                Some(0)
            ))
        );
        insta::assert_snapshot!(
            "test_encode_nvenc",
            render(&test_encode_args(Encoder::H264Nvenc))
        );
        insta::assert_snapshot!(
            "test_encode_vaapi",
            render(&test_encode_args(Encoder::H264Vaapi))
        );
        insta::assert_snapshot!("probe", render(&probe_args(Path::new("in.mp4"))));
        insta::assert_snapshot!(
            "probe_frames",
            render(&probe_frames_args(Path::new("in.mp4")))
        );
        insta::assert_snapshot!(
            "decode_first",
            render(&decode_check_args(Path::new("out.mp4"), Edge::First))
        );
        insta::assert_snapshot!(
            "decode_last",
            render(&decode_check_args(Path::new("out.mp4"), Edge::Last))
        );
        insta::assert_snapshot!(
            "extract_frame",
            render(&extract_frame_args(Path::new("in.mp4"), 12_345))
        );
        insta::assert_snapshot!(
            "decode_image",
            render(&decode_image_args(Path::new("photo.heic")))
        );
    }

    #[test]
    fn partial_names() {
        assert_eq!(
            partial_path(Path::new("clip.mp4")),
            PathBuf::from("clip.partial.mp4")
        );
        assert_eq!(
            partial_path(Path::new("Grandkids 🎂 誕生日 (final).webm")),
            PathBuf::from("Grandkids 🎂 誕生日 (final).partial.webm")
        );
        assert_eq!(
            partial_path(Path::new("noext")),
            PathBuf::from("noext.partial")
        );
    }

    #[test]
    fn fps_and_seconds_formatting() {
        assert_eq!(fps_arg(30.0), "30");
        assert_eq!(fps_arg(29.97), "29.97");
        assert_eq!(fps_arg(23.976), "23.976");
        assert_eq!(fps_arg(60.0), "60");
        assert_eq!(seconds(2_500), "2.500");
        assert_eq!(seconds(0), "0.000");
        assert_eq!(seconds(61_007), "61.007");
    }

    #[test]
    fn paths_pass_through_untouched() {
        let weird = Path::new("Grandkids 🎂 誕生日 (final).mov");
        let args = probe_args(weird);
        assert_eq!(args.last().unwrap(), weird.as_os_str());
        let mut j = job(Encoder::H264Nvenc, Pass::Single);
        j.input = weird;
        let args = encode_args(&j);
        assert!(args.iter().any(|a| a == weird.as_os_str()));
    }

    #[test]
    fn encoder_names_round_trip() {
        for e in Encoder::ALL {
            assert_eq!(Encoder::parse(e.name()), Some(e));
        }
        assert_eq!(Encoder::parse("libx265"), None);
        assert_eq!(
            Encoder::H264Mf { hw_encoding: false }.attempt_label(),
            "h264_mf-sw"
        );
        assert_eq!(Encoder::LibvpxVp9.attempt_label(), "libvpx-vp9-2pass");
    }
}
