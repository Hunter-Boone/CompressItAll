/**
 * @cia/webvideo: the browser video pipeline (DESIGN.md 3.5.10). mediabunny
 * (MPL-2.0, used unmodified) demuxes and muxes; WebCodecs decodes and encodes.
 * The planner math stays in Rust (`cia-video-plan` through the wasm engine);
 * this package probes, transcodes, verifies and extracts preview frames, and
 * runs the 3.5.8 retry loop around a `retry_video` callback the host wires to
 * the engine. Everything here works in a dedicated worker.
 */
export { probeVideo, ProbeError, type ProbeResult } from "./probe";
export { encoderSupport, canEncodePlan, videoCodecString, audioCodecString, type EncoderSupport } from "./capabilities";
export { transcode, remux, TranscodeError, type TranscodeHandle, type TranscodeRequest, type RemuxRequest } from "./transcode";
export { verifyOutput, type ExpectedOutput } from "./verify";
export { retryLoop, MAX_SIZE_ATTEMPTS, type RetryDeps, type RetryOutcome } from "./retry";
export { FrameExtractor } from "./frame";
export { HDR_VERIFIED_BROWSERS, isHdrVerified } from "./hdr-verified";
export { mixAudioTracks, type MixOptions } from "./mix";
