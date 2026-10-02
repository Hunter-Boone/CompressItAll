/**
 * The video row of the honesty table (DESIGN.md 3.11) for bytes WebCodecs
 * produced: re-open with mediabunny, check container and codecs against the
 * preset, duration within max(0.1 s, one frame), aspect within 1 percent,
 * portrait stays portrait, no rotation metadata, audio present iff planned,
 * size strictly under the hard limit, and decode the first and last second
 * with `VideoDecoder`.
 */
import { ALL_FORMATS, BufferSource, Input, VideoSampleSink } from "mediabunny";
import type { AudioCodec, Container, VerificationReport, VideoCodec, VideoFormat } from "@cia/engine-client";
import { audioCodecToken, containerToken, videoCodecToken } from "./probe";

export interface ExpectedOutput {
  container: Container;
  video_codec: VideoCodec;
  audio_codec: AudioCodec;
  duration_ms: number;
  fps: number;
  width: number;
  height: number;
  has_audio: boolean;
  /** null in Smaller mode. */
  hard_bytes: number | null;
  /** The preset's formats; empty means anything the planner produces is fine. */
  allowed: VideoFormat[];
}

export async function verifyOutput(bytes: Uint8Array, expected: ExpectedOutput): Promise<VerificationReport> {
  const r: VerificationReport = { size_ok: false, decodes: false, checks: [], failures: [] };
  const size = bytes.byteLength;
  if (expected.hard_bytes === null || size < expected.hard_bytes) {
    r.size_ok = true;
    r.checks.push(expected.hard_bytes === null ? `size ${size}` : `size ${size} < ${expected.hard_bytes}`);
  } else {
    r.failures.push(`size ${size} >= ${expected.hard_bytes}`);
  }

  const input = new Input({ formats: ALL_FORMATS, source: new BufferSource(bytes) });
  try {
    let container: string;
    try {
      container = containerToken((await input.getFormat()).name);
    } catch (e) {
      r.failures.push(`mediabunny cannot read the output: ${e instanceof Error ? e.message : String(e)}`);
      return r;
    }
    r.checks.push("mediabunny reads it");
    if (container === expected.container) r.checks.push(`container ${container}`);
    else r.failures.push(`container ${container} not ${expected.container}`);

    const video = await input.getPrimaryVideoTrack();
    if (!video) {
      r.failures.push("no video track");
      return r;
    }
    const codec = videoCodecToken(video.codec);
    if (codec === expected.video_codec) r.checks.push(`codec ${codec}`);
    else r.failures.push(`codec ${codec} not ${expected.video_codec}`);
    if (expected.allowed.length) {
      const ok = expected.allowed.some((f) => f.container === container && f.video === codec);
      if (ok) r.checks.push("container and codec allowed by the preset");
      else r.failures.push(`${container}/${codec} is not a format the preset accepts`);
    }

    const durationMs = Math.round((await input.computeDuration()) * 1000);
    const tol = Math.max(100, Math.round(1000 / Math.max(1, expected.fps)));
    const diff = Math.abs(durationMs - expected.duration_ms);
    if (diff <= tol) r.checks.push(`duration ${durationMs} ms within ${tol} ms`);
    else r.failures.push(`duration ${durationMs} ms is ${diff} ms from ${expected.duration_ms} (tolerance ${tol})`);

    const w = video.displayWidth;
    const h = video.displayHeight;
    if (expected.width > 0 && expected.height > 0 && w > 0 && h > 0) {
      const want = expected.width / expected.height;
      const got = w / h;
      if (Math.abs((got - want) / want) <= 0.01) r.checks.push(`aspect ${w}x${h}`);
      else r.failures.push(`aspect ${w}x${h} differs from ${expected.width}x${expected.height} by more than 1 percent`);
      const wantPortrait = expected.height > expected.width;
      if (wantPortrait && !(h > w)) r.failures.push("portrait source came out landscape");
      else if (wantPortrait) r.checks.push("portrait stays portrait");
      if (video.rotation !== 0) r.failures.push(`output carries rotation ${video.rotation}`);
    } else {
      r.failures.push("no picture size");
    }

    const audio = await input.getAudioTracks();
    if (audio.length > 0 === expected.has_audio) {
      r.checks.push(audio.length ? `${audio.length} audio stream(s)` : "no audio, as planned");
      if (expected.has_audio && audio.length !== 1) r.failures.push(`${audio.length} audio streams, expected 1`);
      if (expected.has_audio && audio[0]) {
        const ac = audioCodecToken(audio[0].codec);
        if (ac === expected.audio_codec) r.checks.push(`audio ${ac}`);
        else r.failures.push(`audio ${ac} not ${expected.audio_codec}`);
      }
    } else {
      r.failures.push(audio.length ? "audio present but none planned" : "audio missing");
    }

    // First and last second through VideoDecoder.
    try {
      const sink = new VideoSampleSink(video);
      const total = durationMs / 1000;
      let first = 0;
      for await (const s of sink.samples(0, Math.min(1, total))) { first++; s.close(); }
      let last = 0;
      for await (const s of sink.samples(Math.max(0, total - 1), total)) { last++; s.close(); }
      if (first > 0 && last > 0) {
        r.decodes = true;
        r.checks.push(`first second decodes (${first} frames), last second decodes (${last} frames)`);
      } else {
        r.failures.push(`decode produced ${first} frames in the first second and ${last} in the last`);
      }
    } catch (e) {
      r.failures.push(`decode failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  } finally {
    input.dispose();
  }
  return r;
}
