/**
 * Preview frames for the Trim panel (DESIGN.md 4.4): decode the frame at a
 * time with `VideoDecoder` through mediabunny (which seeks to the nearest key
 * frame and decodes forward), draw it into an `OffscreenCanvas` and return a
 * JPEG data URL. One extractor per file keeps the decoder warm across the
 * debounced drag requests.
 */
import { ALL_FORMATS, BlobSource, Input, VideoSampleSink, type InputVideoTrack } from "mediabunny";

export class FrameExtractor {
  private input: Input;
  private ready: Promise<{ track: InputVideoTrack; sink: VideoSampleSink } | null>;
  private canvas: OffscreenCanvas | null = null;

  constructor(file: Blob) {
    this.input = new Input({ formats: ALL_FORMATS, source: new BlobSource(file) });
    this.ready = this.input.getPrimaryVideoTrack().then((track) => (track ? { track, sink: new VideoSampleSink(track) } : null)).catch(() => null);
  }

  /** JPEG data URL of the frame at `ms`, scaled to `maxWidth`; null when nothing decodes. */
  async frame(ms: number, maxWidth = 320, quality = 0.8): Promise<string | null> {
    const r = await this.ready;
    if (!r) return null;
    const sample = await r.sink.getSample(Math.max(0, ms) / 1000).catch(() => null);
    if (!sample) return null;
    try {
      const w = sample.displayWidth || r.track.displayWidth;
      const h = sample.displayHeight || r.track.displayHeight;
      const scale = Math.min(1, maxWidth / Math.max(1, w));
      const cw = Math.max(1, Math.round(w * scale));
      const ch = Math.max(1, Math.round(h * scale));
      if (!this.canvas || this.canvas.width !== cw || this.canvas.height !== ch) this.canvas = new OffscreenCanvas(cw, ch);
      const ctx = this.canvas.getContext("2d");
      if (!ctx) return null;
      sample.draw(ctx, 0, 0, cw, ch);
      const blob = await this.canvas.convertToBlob({ type: "image/jpeg", quality });
      return `data:image/jpeg;base64,${toBase64(new Uint8Array(await blob.arrayBuffer()))}`;
    } finally {
      sample.close();
    }
  }

  dispose() {
    this.input.dispose();
  }
}

function toBase64(bytes: Uint8Array): string {
  let s = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) s += String.fromCharCode(...bytes.subarray(i, i + chunk));
  return btoa(s);
}
