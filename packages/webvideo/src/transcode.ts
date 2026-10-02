/**
 * Transcode or remux one video with mediabunny + WebCodecs (DESIGN.md 3.5.10
 * step 3). The plan comes from the Rust planner; this file only executes it:
 * exact output size and frame rate, planned bitrates, key frames every 2 s,
 * rotation baked into the frames (no rotation metadata in the output,
 * 3.5.7), metadata stripped, trim applied in source time, audio chosen,
 * removed or mixed per `AudioTrackChoice`.
 */
import {
  ALL_FORMATS,
  AudioSampleSource,
  BlobSource,
  BufferTarget,
  Conversion,
  ConversionCanceledError,
  Input,
  Mp4OutputFormat,
  Output,
  WebMOutputFormat,
  type ConversionAudioOptions,
  type ConversionVideoOptions,
  type InputAudioTrack,
} from "mediabunny";
import type { AudioTrackChoice, Container, VideoPlan, VideoProbe } from "@cia/engine-client";
import { mixAudioTracks } from "./mix";

export class TranscodeError extends Error {
  constructor(message: string, readonly code: "cancelled" | "browser_lacks_codec" | "damaged" | "encoder_crash") {
    super(message);
  }
}

export interface TranscodeRequest {
  file: Blob;
  probe: VideoProbe;
  plan: VideoPlan;
  /** Source range in ms. */
  trim: [number, number] | null;
  audio: AudioTrackChoice;
  onProgress?: (fraction: number) => void;
}

export interface RemuxRequest {
  file: Blob;
  probe: VideoProbe;
  container: Container;
  trim: [number, number] | null;
  audio: AudioTrackChoice;
  onProgress?: (fraction: number) => void;
}

export interface TranscodeHandle {
  done: Promise<Uint8Array<ArrayBuffer>>;
  cancel(): Promise<void>;
}

/** Output up to this size is muxed in memory with `fastStart: "in-memory"` (3.5.10). */
export const IN_MEMORY_LIMIT = 1 << 30;

function outputFormat(container: Container) {
  return container === "mp4" ? new Mp4OutputFormat({ fastStart: "in-memory" }) : new WebMOutputFormat();
}

function trimSeconds(trim: [number, number] | null, probe: VideoProbe): { start: number; end: number } | undefined {
  if (!trim) return undefined;
  const total = Number(probe.duration_ms) / 1000;
  const start = Math.max(0, trim[0] / 1000);
  const end = Math.min(total, trim[1] / 1000);
  return end > start ? { start, end } : undefined;
}

/** Which source audio tracks feed the output: none, one, or all (mixed). */
function audioSelection(probe: VideoProbe, choice: AudioTrackChoice): number[] {
  if (choice.type === "remove" || probe.audio.length === 0) return [];
  if (choice.type === "track" && probe.audio.some((a) => a.index === choice.index)) return [choice.index];
  return probe.audio.map((a) => a.index);
}

function wrap(e: unknown): TranscodeError {
  if (e instanceof TranscodeError) return e;
  if (e instanceof ConversionCanceledError) return new TranscodeError("cancelled", "cancelled");
  const msg = e instanceof Error ? e.message : String(e);
  if (/decod|unsupported|not supported|config/i.test(msg)) return new TranscodeError(msg, "browser_lacks_codec");
  if (/EOF|end of|truncat|out of bounds|invalid|corrupt|malformed/i.test(msg)) return new TranscodeError(msg, "damaged");
  return new TranscodeError(msg, "encoder_crash");
}

export function transcode(req: TranscodeRequest): TranscodeHandle {
  const { plan, probe } = req;
  let conversion: Conversion | null = null;
  let cancelled = false;
  const input = new Input({ formats: ALL_FORMATS, source: new BlobSource(req.file) });
  const target = new BufferTarget();
  const output = new Output({ format: outputFormat(plan.container), target });
  const trim = trimSeconds(req.trim, probe);
  const selected = audioSelection(probe, req.audio);
  const wantAudio = plan.audio_bps > 0n && selected.length > 0;
  const audioCodec = plan.audio_codec === "opus" ? "opus" : "aac";
  const channels = Math.max(1, plan.audio_channels);
  const audioBitrate = Number(plan.audio_bps);

  const video: ConversionVideoOptions = {
    width: plan.width,
    height: plan.height,
    fit: "fill",
    codec: plan.video_codec === "vp9" ? "vp9" : plan.video_codec === "av1" ? "av1" : "avc",
    bitrate: Number(plan.video_bps),
    frameRate: plan.fps,
    keyFrameInterval: 2,
    forceTranscode: true,
    allowTransformationMetadata: false,
    hardwareAcceleration: "no-preference",
  };

  const done = (async () => {
    try {
      const mixAll = wantAudio && selected.length > 1;
      let audio: ConversionAudioOptions | ((track: InputAudioTrack, n: number) => ConversionAudioOptions);
      if (!wantAudio || mixAll) {
        audio = { discard: true };
      } else {
        const chosen = selected[0]! + 1; // mediabunny numbers audio tracks from 1
        audio = (_track, n) => (n === chosen ? { codec: audioCodec, bitrate: audioBitrate, numberOfChannels: channels, forceTranscode: true } : { discard: true });
      }
      conversion = await Conversion.init({ input, output, video, audio, trim, tags: {}, showWarnings: false, composable: mixAll });
      if (cancelled) throw new ConversionCanceledError();
      if (!conversion.isValid || !conversion.utilizedTracks.some((t) => t.isVideoTrack())) {
        const reasons = conversion.discardedTracks.map((d) => `${d.track.type}: ${d.reason}`).join(", ");
        const codecIssue = conversion.discardedTracks.some((d) => d.reason === "undecodable_source_codec" || d.reason === "no_encodable_target_codec" || d.reason === "unknown_source_codec");
        throw new TranscodeError(`conversion not possible (${reasons})`, codecIssue ? "browser_lacks_codec" : "damaged");
      }
      conversion.onProgress = (p) => req.onProgress?.(Math.max(0, Math.min(1, p)));
      if (mixAll) {
        const tracks = await input.getAudioTracks();
        const chosen = selected.map((i) => tracks[i]).filter((t): t is InputAudioTrack => !!t);
        const source = new AudioSampleSource({ codec: audioCodec, bitrate: audioBitrate });
        output.addAudioTrack(source);
        await output.start();
        const start = trim?.start ?? 0;
        const end = trim?.end ?? Number(probe.duration_ms) / 1000;
        await Promise.all([
          conversion.execute(),
          mixAudioTracks(chosen, (s) => source.add(s).finally(() => s.close()), { start, end, channels, cancelled: () => cancelled }),
        ]);
        source.close();
        if (cancelled) throw new ConversionCanceledError();
        await output.finalize();
      } else {
        await conversion.execute();
      }
      if (!target.buffer) throw new TranscodeError("no output produced", "encoder_crash");
      return new Uint8Array(target.buffer);
    } catch (e) {
      throw wrap(e);
    } finally {
      input.dispose();
    }
  })();

  return {
    done,
    cancel: async () => {
      cancelled = true;
      await conversion?.cancel().catch(() => {});
      if (output.state === "started") await output.cancel().catch(() => {});
    },
  };
}

/** Copy the streams into another container (DESIGN.md 3.5.4, "remux with -c copy"). */
export function remux(req: RemuxRequest): TranscodeHandle {
  let conversion: Conversion | null = null;
  let cancelled = false;
  const input = new Input({ formats: ALL_FORMATS, source: new BlobSource(req.file) });
  const target = new BufferTarget();
  const output = new Output({ format: outputFormat(req.container), target });
  const selected = audioSelection(req.probe, req.audio);
  const done = (async () => {
    try {
      const chosen = selected.length ? selected[0]! + 1 : -1;
      conversion = await Conversion.init({
        input,
        output,
        video: { forceTranscode: false },
        audio: (_t, n) => (n === chosen ? {} : { discard: true }),
        trim: trimSeconds(req.trim, req.probe),
        tags: {},
        copy: { mode: "forced" },
        showWarnings: false,
      });
      if (cancelled) throw new ConversionCanceledError();
      if (!conversion.isValid) throw new TranscodeError("cannot copy streams into this container", "encoder_crash");
      conversion.onProgress = (p) => req.onProgress?.(p);
      await conversion.execute();
      if (!target.buffer) throw new TranscodeError("no output produced", "encoder_crash");
      return new Uint8Array(target.buffer);
    } catch (e) {
      throw wrap(e);
    } finally {
      input.dispose();
    }
  })();
  return {
    done,
    cancel: async () => {
      cancelled = true;
      await conversion?.cancel().catch(() => {});
    },
  };
}
