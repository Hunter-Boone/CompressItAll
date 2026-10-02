/**
 * Probe a video with mediabunny into the `VideoProbe` shape the Rust planner
 * reads (DESIGN.md 3.5.1): duration, container, codec, coded and display
 * size after rotation, rotation, average and peak frame rate, VFR from packet
 * timing over the first seconds, colour transfer and primaries, HDR flag and
 * the audio tracks. Also reports whether this browser's `VideoDecoder` takes
 * the codec (3.5.10 step 1), which the planner turns into `BrowserLacksCodec`.
 */
import { ALL_FORMATS, BlobSource, EncodedPacketSink, Input, type InputAudioTrack, type InputVideoTrack } from "mediabunny";
import type { AudioTrack, VideoProbe } from "@cia/engine-client";
import { canDecodeConfig } from "./capabilities";

export class ProbeError extends Error {
  constructor(message: string, readonly kind: "damaged" | "no_video") {
    super(message);
  }
}

export interface ProbeResult {
  probe: VideoProbe;
  /** `VideoDecoder.isConfigSupported` for the source track. */
  canDecode: boolean;
  codecString: string | null;
}

/** Lower-case tokens as ffprobe names them, so both hosts feed the planner the same words. */
export function containerToken(formatName: string): string {
  switch (formatName) {
    case "MP4": return "mp4";
    case "QuickTime File Format": return "mov";
    case "Matroska": return "mkv";
    case "WebM": return "webm";
    case "MPEG Transport Stream": return "ts";
    case "Ogg": return "ogv";
    default: return formatName.toLowerCase().replace(/\s+/g, "_");
  }
}

export function videoCodecToken(codec: InputVideoTrack["codec"]): string {
  switch (codec) {
    case "avc": return "h264";
    case null: return "unknown";
    default: return codec;
  }
}

export function audioCodecToken(codec: InputAudioTrack["codec"]): string {
  switch (codec) {
    case null: return "unknown";
    case "ulaw": return "pcm_mulaw";
    case "alaw": return "pcm_alaw";
    default:
      if (codec.startsWith("pcm-")) {
        const rest = codec.slice(4);
        return rest.endsWith("be") ? `pcm_${rest}` : rest === "u8" || rest === "s8" ? `pcm_${rest}` : `pcm_${rest}le`;
      }
      return codec;
  }
}

function transferToken(t: string | undefined): string {
  switch (t) {
    case "pq": return "smpte2084";
    case "hlg": return "arib-std-b67";
    case "iec61966-2-1": return "iec61966-2-1";
    case undefined: return "";
    default: return t;
  }
}

/** Packets to look at for frame-rate statistics: five seconds at 60 fps. */
const FPS_PACKETS = 300;
/** More than this spread in frame timing over the probed packets means VFR (3.5.1). */
const VFR_SPREAD = 0.02;

export async function probeVideo(file: Blob): Promise<ProbeResult> {
  const input = new Input({ formats: ALL_FORMATS, source: new BlobSource(file) });
  try {
    let formatName: string;
    try {
      formatName = (await input.getFormat()).name;
    } catch (e) {
      throw new ProbeError(`unreadable container: ${e instanceof Error ? e.message : String(e)}`, "damaged");
    }
    let video: InputVideoTrack | null;
    try {
      video = await input.getPrimaryVideoTrack();
    } catch (e) {
      throw new ProbeError(`no readable track list: ${e instanceof Error ? e.message : String(e)}`, "damaged");
    }
    if (!video) throw new ProbeError("no video track", "no_video");

    let durationS: number;
    let fps: Awaited<ReturnType<InputVideoTrack["computeFrameRateMetrics"]>>;
    try {
      [durationS, fps] = await Promise.all([input.computeDuration(), video.computeFrameRateMetrics({ targetPacketCount: FPS_PACKETS })]);
      // Read the first and the last packet: a truncated file has a usable index but no bytes behind it.
      const packets = new EncodedPacketSink(video);
      const first = await packets.getFirstPacket();
      const last = await packets.getPacket(Math.max(0, durationS - 0.001));
      if (!first || !last) throw new Error("no packets");
    } catch (e) {
      throw new ProbeError(`packets unreadable: ${e instanceof Error ? e.message : String(e)}`, "damaged");
    }
    if (!(durationS > 0) || video.displayWidth === 0 || video.displayHeight === 0) throw new ProbeError("no duration or picture size", "damaged");

    const colour = await video.getColorSpace().catch(() => ({} as VideoColorSpaceInit));
    // lib.dom's VideoTransferCharacteristics predates the HDR values WebCodecs and mediabunny report.
    const transfer = (colour.transfer ?? undefined) as string | undefined;
    const hdr = (await video.hasHighDynamicRange().catch(() => false)) || transfer === "pq" || transfer === "hlg";
    const audioTracks = await input.getAudioTracks();
    const audio: AudioTrack[] = [];
    for (const [i, a] of audioTracks.entries()) {
      const bitrate = await a.getBitrate().catch(() => null);
      audio.push({
        index: i,
        codec: audioCodecToken(a.codec),
        channels: a.numberOfChannels,
        sample_rate: a.sampleRate,
        bitrate_bps: bitrate === null ? null : BigInt(Math.round(bitrate)),
        title: a.name,
      });
    }
    const avg = fps.averageFrameRate > 0 ? fps.averageFrameRate : fps.bestGuessFrameRate;
    const spread = avg > 0 ? (fps.maxFrameRate - fps.minFrameRate) / avg : 0;
    const probe: VideoProbe = {
      duration_ms: BigInt(Math.round(durationS * 1000)),
      container: containerToken(formatName),
      video_codec: videoCodecToken(video.codec),
      coded_w: video.codedWidth,
      coded_h: video.codedHeight,
      display_w: video.displayWidth,
      display_h: video.displayHeight,
      rotation_degrees: video.rotation,
      avg_fps: avg,
      max_fps: Math.max(fps.maxFrameRate, avg),
      is_vfr: !fps.frameRateIsConstant && spread > VFR_SPREAD,
      pixel_format: "",
      color_transfer: transferToken(transfer),
      color_primaries: colour.primaries ?? "",
      is_hdr: hdr,
      dolby_vision_profile: null,
      audio,
    };
    const [canDecode, codecString] = await Promise.all([video.getDecoderConfig().then(canDecodeConfig).catch(() => false), video.getCodecParameterString().catch(() => null)]);
    return { probe, canDecode, codecString };
  } finally {
    input.dispose();
  }
}
