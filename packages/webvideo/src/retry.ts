/**
 * The 3.5.8 loop around a WebCodecs encode: encode, measure, verify; on an
 * overshoot ask the Rust planner (`retry_video`, hardware factor 0.93) for the
 * next plan and go again, at most four attempts. An undershoot is accepted
 * as is. Every attempt is reported for the job log.
 */
import type { Attempt, VerificationReport, VideoPlan } from "@cia/engine-client";
import { TranscodeError } from "./transcode";

export const MAX_SIZE_ATTEMPTS = 4;

export interface RetryDeps {
  encode(plan: VideoPlan, attempt: number): Promise<Uint8Array<ArrayBuffer>>;
  verify(bytes: Uint8Array, plan: VideoPlan): Promise<VerificationReport>;
  /** `cia_wasm::retry_video`: the next plan, or null when no rung is left. */
  retry(plan: VideoPlan, actualBytes: number): Promise<VideoPlan | null>;
  onAttempt?(attempt: Attempt): void;
  cancelled?(): boolean;
}

export type RetryOutcome =
  | { type: "fitted"; bytes: Uint8Array<ArrayBuffer>; plan: VideoPlan; verification: VerificationReport; attempts: Attempt[] }
  | { type: "over"; closest: number; attempts: Attempt[] }
  | { type: "cancelled"; attempts: Attempt[] }
  | { type: "failed"; code: string; message: string; closest: number | null; attempts: Attempt[] };

function params(plan: VideoPlan): Record<string, unknown> {
  return {
    encoder: "webcodecs",
    two_pass: false,
    width: plan.width,
    height: plan.height,
    fps: plan.fps,
    video_bps: Number(plan.video_bps),
    audio_bps: Number(plan.audio_bps),
    audio_channels: plan.audio_channels,
    container: plan.container,
    rung_index: plan.rung_index,
    predicted_bytes: Number(plan.predicted_bytes),
    quality: plan.quality,
  };
}

export async function retryLoop(initial: VideoPlan, hardBytes: number | null, itemId: string, deps: RetryDeps, maxAttempts = MAX_SIZE_ATTEMPTS): Promise<RetryOutcome> {
  const attempts: Attempt[] = [];
  let plan = initial;
  let closest: number | null = null;
  for (let n = 1; ; n++) {
    if (deps.cancelled?.()) return { type: "cancelled", attempts };
    const started = Date.now();
    const attempt: Attempt = { n, item_id: itemId, encoder: `webcodecs-${plan.video_codec}`, params: params(plan), output_bytes: null, score: null, elapsed_ms: 0n, verdict: { type: "cancelled" } };
    const report = (a: Attempt) => { a.elapsed_ms = BigInt(Date.now() - started); attempts.push(a); deps.onAttempt?.(a); };
    let bytes: Uint8Array<ArrayBuffer>;
    try {
      bytes = await deps.encode(plan, n);
    } catch (e) {
      if (e instanceof TranscodeError && e.code === "cancelled") {
        report(attempt);
        return { type: "cancelled", attempts };
      }
      const message = e instanceof Error ? e.message : String(e);
      attempt.verdict = { type: "error", message };
      report(attempt);
      return { type: "failed", code: e instanceof TranscodeError ? e.code : "encoder_crash", message, closest, attempts };
    }
    const actual = bytes.byteLength;
    attempt.output_bytes = BigInt(actual);
    closest = closest === null ? actual : Math.min(closest, actual);
    const verification = await deps.verify(bytes, plan);
    const passed = verification.size_ok && verification.decodes && verification.failures.length === 0;
    if (passed && (hardBytes === null || actual < hardBytes)) {
      attempt.verdict = { type: "fits" };
      report(attempt);
      return { type: "fitted", bytes, plan, verification, attempts };
    }
    if (!passed && verification.size_ok) {
      attempt.verdict = { type: "error", message: `verification failed: ${verification.failures.join("; ")}` };
      report(attempt);
      return { type: "failed", code: "verify_failed", message: verification.failures.join("; "), closest, attempts };
    }
    attempt.verdict = { type: "over", by_bytes: BigInt(Math.max(0, actual - (hardBytes ?? actual))) };
    if (n >= maxAttempts) {
      report(attempt);
      return { type: "over", closest: closest ?? actual, attempts };
    }
    const next = await deps.retry(plan, actual);
    if (!next) {
      attempt.verdict = { type: "below_floor" };
      report(attempt);
      return { type: "over", closest: closest ?? actual, attempts };
    }
    report(attempt);
    plan = next;
  }
}
