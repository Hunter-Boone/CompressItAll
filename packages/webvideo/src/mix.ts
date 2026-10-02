/**
 * Mix several audio tracks into one (DESIGN.md 3.5.3, 3.5.10 step 5): decode
 * each with `AudioDecoder` (through mediabunny's `AudioSampleSink`), sum them
 * at 48 kHz in one-second windows, clip at 0.97 full scale (the stand-in for
 * `alimiter=limit=0.97`), and hand the mixed samples to the output. Runs in
 * the worker without an AudioWorklet.
 */
import { AudioSample, AudioSampleSink, type InputAudioTrack } from "mediabunny";

export interface MixOptions {
  /** Source time range in seconds. */
  start: number;
  end: number;
  channels: number;
  sampleRate?: number;
  limit?: number;
  cancelled?: () => boolean;
}

const WINDOW_S = 1;

export async function mixAudioTracks(tracks: InputAudioTrack[], push: (sample: AudioSample) => Promise<void>, opts: MixOptions): Promise<void> {
  const sr = opts.sampleRate ?? 48_000;
  const channels = Math.max(1, opts.channels);
  const limit = opts.limit ?? 0.97;
  const sinks = tracks.map((t) => new AudioSampleSink(t));
  for (let t = opts.start; t < opts.end; t += WINDOW_S) {
    if (opts.cancelled?.()) return;
    const windowEnd = Math.min(t + WINDOW_S, opts.end);
    const frames = Math.round((windowEnd - t) * sr);
    if (frames <= 0) break;
    const mix = new Float32Array(frames * channels);
    for (const sink of sinks) {
      for await (const s of sink.samples(t, windowEnd)) {
        try {
          addSample(mix, frames, channels, sr, t, s);
        } finally {
          s.close();
        }
      }
    }
    for (let i = 0; i < mix.length; i++) {
      const v = mix[i]!;
      mix[i] = v > limit ? limit : v < -limit ? -limit : v;
    }
    await push(new AudioSample({ data: mix, format: "f32", numberOfChannels: channels, sampleRate: sr, timestamp: t - opts.start }));
  }
}

/** Add one decoded sample into the interleaved window buffer, resampling linearly when the rates differ. */
function addSample(mix: Float32Array, frames: number, channels: number, sr: number, windowStart: number, s: AudioSample) {
  const srcCh = s.numberOfChannels;
  const n = s.numberOfFrames;
  const planes: Float32Array[] = [];
  for (let c = 0; c < srcCh; c++) {
    const plane = new Float32Array(n);
    s.copyTo(plane, { planeIndex: c, format: "f32-planar" });
    planes.push(plane);
  }
  const ratio = s.sampleRate / sr;
  const offset = (s.timestamp - windowStart) * sr; // output frame index of the sample's first frame
  const firstOut = Math.max(0, Math.ceil(offset));
  const lastOut = Math.min(frames - 1, Math.floor(offset + (n - 1) / ratio));
  for (let o = firstOut; o <= lastOut; o++) {
    const pos = (o - offset) * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(n - 1, i0 + 1);
    const frac = pos - i0;
    for (let c = 0; c < channels; c++) {
      // Mono sources go to every output channel; wider sources map channel by channel.
      const plane = planes[srcCh === 1 ? 0 : Math.min(c, srcCh - 1)]!;
      const v = plane[i0]! * (1 - frac) + plane[i1]! * frac;
      const idx = o * channels + c;
      mix[idx] = (mix[idx] ?? 0) + v;
    }
  }
}
