/**
 * WebCodecs capability checks (DESIGN.md 3.5.10 step 1). Static checks run at
 * start-up and feed `Capabilities.web.*`; the per-plan check runs before an
 * encode at the planned size and bitrate. Works on the main thread and in
 * workers (`globalThis`).
 */
import type { AudioCodec, VideoCodec, VideoPlan } from "@cia/engine-client";

export interface EncoderSupport {
  webcodecs: boolean;
  h264: boolean;
  vp9: boolean;
  aac: boolean;
  opus: boolean;
}

type Supported = { supported?: boolean };
interface VideoEncoderStatic { isConfigSupported(c: VideoEncoderConfig): Promise<Supported> }
interface AudioEncoderStatic { isConfigSupported(c: AudioEncoderConfig): Promise<Supported> }
interface VideoDecoderStatic { isConfigSupported(c: VideoDecoderConfig): Promise<Supported> }

const g = globalThis as unknown as { VideoEncoder?: VideoEncoderStatic; AudioEncoder?: AudioEncoderStatic; VideoDecoder?: VideoDecoderStatic };

async function sup(p: Promise<Supported> | undefined): Promise<boolean> {
  try {
    return !!(await p)?.supported;
  } catch {
    return false;
  }
}

/** H.264 High 4.0 (3.5.10) and VP9 profile 0 level 4.0 8-bit. */
export function videoCodecString(codec: VideoCodec): string {
  return codec === "vp9" ? "vp09.00.40.08" : codec === "av1" ? "av01.0.08M.08" : "avc1.640028";
}

export function audioCodecString(codec: AudioCodec): string {
  return codec === "opus" ? "opus" : "mp4a.40.2";
}

function videoConfig(codec: string, width: number, height: number, bitrate: number, framerate: number): VideoEncoderConfig {
  return { codec, width, height, bitrate, framerate, hardwareAcceleration: "no-preference" };
}

/** Encoders this browser has, probed at 1080p30 / 5 Mb/s and 48 kHz stereo 128 kb/s. */
export async function encoderSupport(): Promise<EncoderSupport> {
  const video = (codec: string) => (g.VideoEncoder ? sup(g.VideoEncoder.isConfigSupported(videoConfig(codec, 1920, 1080, 5_000_000, 30))) : Promise.resolve(false));
  const audio = (codec: string) => (g.AudioEncoder ? sup(g.AudioEncoder.isConfigSupported({ codec, sampleRate: 48_000, numberOfChannels: 2, bitrate: 128_000 })) : Promise.resolve(false));
  const [h264, vp9, aac, opus] = await Promise.all([video("avc1.640028"), video("vp09.00.40.08"), audio("mp4a.40.2"), audio("opus")]);
  return { webcodecs: !!g.VideoEncoder && !!g.VideoDecoder, h264, vp9, aac, opus };
}

/** Does `VideoDecoder` take this track's configuration? */
export async function canDecodeConfig(config: VideoDecoderConfig | null): Promise<boolean> {
  if (!config || !g.VideoDecoder) return false;
  return sup(g.VideoDecoder.isConfigSupported(config));
}

/** `VideoEncoder.isConfigSupported` for the planned codec at the planned size, bitrate and frame rate. */
export async function canEncodePlan(plan: Pick<VideoPlan, "video_codec" | "width" | "height" | "video_bps" | "fps">): Promise<boolean> {
  if (!g.VideoEncoder) return false;
  return sup(g.VideoEncoder.isConfigSupported(videoConfig(videoCodecString(plan.video_codec), plan.width, plan.height, Number(plan.video_bps), plan.fps)));
}
