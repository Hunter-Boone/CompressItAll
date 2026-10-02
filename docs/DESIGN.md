# Smidge (working folder: CompressItAll): design document

Status: architecture decided, ready for implementation. Author: architect agent, 2026-10-02.
Audience: the implementing agent (who follows this verbatim) and Hunter (owner).

Conventions in this document:

- "MUST" means the implementer has no discretion. Everything else is still a decision, but the implementer may adjust details that this document does not pin down (variable names, file splits inside a module) as long as behaviour matches.
- Byte sizes are exact integers. `MB` means 1,000,000 bytes and `MiB` means 1,048,576 bytes. The UI always shows decimal MB with one decimal place.
- Internal code names use the prefix `cia` (from the folder name) so a product rename never touches crate or package names. The product name lives in exactly one place: `packages/brand/brand.json`.
- Every external fact (attachment limits, licenses, browser support) was checked on 2026-10-02. Sources are listed next to the fact.

## Contents

1. Product
2. Architecture
3. The engine
4. UI and UX
5. Auth, licensing and payments
6. Distribution
7. Testing
8. Milestones
9. Open questions for Hunter
10. Third-party inventory

---

# 1. Product

## 1.1 Name

**Smidge.** A smidge is a tiny amount. It is short, friendly, easy to spell after hearing it once in a YouTube tutorial, and says "small" without sounding technical. A web search on 2026-10-02 found no consumer file-compression product with that name; the only hit was an ASP.NET bundling library (Shazwazza/Smidge), which is a different market. A trademark search and the domain purchase are Hunter's (section 9).

Alternates, in order of preference:

1. **Tuck** ("tuck it into an email"). Very short; harder to search for.
2. **Sendsize**. Literal and searchable; less warm.
3. **Fitbox**. Says "fits"; slightly generic.

Rejected after checking: Snugfile (an existing "SnugFile" desktop app publishes releases on GitHub), FitSend (an open-source project with exactly this purpose, YuLeo926/FitSend), Pocketsize (an existing browser compressor at pocketsize.vercel.app), FeatherFiles (an existing photo/video optimizer).

## 1.2 The promise

> Drop it in, pick where it's going, and Smidge makes it fit. If it can't, Smidge tells you why and what will work.

The narrow promise is "it fits". Smidge does not promise "any file to any format". The product lesson from ConvertSave's cancellations is built into the engine as a rule: an output that is over the limit is a failure, it is never written as a success, and the UI never shows a success state for it (section 3.11).

Smidge does not contain a new codec, and the marketing must not suggest it does. "Our own compression" means our own size-targeting search and orchestration on top of established, permissively licensed encoders (mozjpeg, libwebp, oxipng, rav1e, zopfli, zstd, xz, LZMA2, Opus, FLAC, and the platform's H.264 encoders). The value is in choosing the right encoder, format, resolution and quality for a byte budget, verifying the result, and refusing honestly when the budget cannot be met. The website and app say this plainly in the About page: "Smidge uses well-known open-source encoders. What Smidge adds is the part that picks the best settings for your limit and checks the result."

## 1.3 User stories

**Grandmother (Margaret, 74, Windows laptop).** Her daughter sent 23 photos of the grandchildren from an iPhone (mixed HEIC and JPEG, about 90 MB). Margaret wants to email them to her sister, and Gmail refused. She opens Smidge, drags the folder onto the window, clicks "Email", and reads "23 photos → about 17 MB in 1 zip file. Fits in one email. Quality: Good." (23 attachments are a lot to handle, so Smidge offers one zip by default above 10 files, section 3.9.2.) She clicks Compress. Smidge writes `Grandkids (Email).zip` next to the folder and shows "Show in folder", and she attaches that one file. Nothing she did needed a technical word. (Her HEIC photos open through Windows' own HEIF decoder; on a PC without it, Smidge offers its one-time download first, section 4.8.) Acceptance: the zip is at most the Email preset's 18.2 MB attachment ceiling (`hard_bytes`, section 3.11), every file opens in Windows Photos and on an iPhone, the photos are upright, and no file is larger than its original.

**Discord gamer (Jayden, 16, Windows desktop, NVIDIA GPU).** He has a 1 min 10 s OBS clip, 1440p60 MKV with two audio tracks (game and mic), 380 MB. Discord free allows 20 MiB. He drops the clip, picks "Discord" (tier "Free" is the default), reads "1 min 10 s video → 720p, 30 fps, about 19.2 MB. Quality: Good.", clicks Compress, then "Copy file", and pastes into Discord, where the clip plays inline with both audio tracks mixed. Acceptance: output is at most 20,905,983 bytes (20 MiB minus the safety margin), H.264 MP4 with faststart, duration within 0.1 s of the source, both tracks audible.

**Office worker (Priya, macOS, Microsoft 365 work email).** Her 64 MB PowerPoint deck has 40 large photos and one embedded MP4. Her company's Exchange rejects it. She drops the .pptx, picks "Work email (Microsoft 365)", reads "Presentation → about 18 MB. Photos re-saved at good quality; 1 video re-encoded." and gets `Q3 Review (Work email).pptx`. Acceptance: the file is under the preset limit, opens in PowerPoint and Keynote, every slide shows its images, the video plays, and the slide XML is untouched.

---
# 2. Architecture

## 2.1 Stack decision

Tauri 2 (Rust backend, system webview) with React 18, TypeScript, Tailwind 3.4 and lucide-react, the same as ConvertSave. Tauri lets the Rust engine run in-process with no Node sidecar, keeps installers around 15 MB instead of Electron's 100 MB or more, and reuses ConvertSave's updater, signing and test lab work; the webview differences between platforms matter little here because all heavy work happens in Rust or in Web Workers, not in the DOM.

Pin `tauri = "2"` (2.11 or later, the version ConvertSave's e2e branch moved to for `tauri-plugin-wdio-webdriver`). Do not adopt Tauri 3 (alpha on crates.io as of 2026-10-01) during v1.

React 18.3 and Tailwind 3.4 are chosen over React 19 and Tailwind 4 so that ConvertSave's components, token files and Tailwind config copy over without porting.

## 2.2 One engine, two hosts

The engine is a Rust workspace compiled two ways:

- natively into the Tauri app (`apps/desktop`), where it can also drive an external FFmpeg process;
- to `wasm32-unknown-unknown` with wasm-bindgen for the web app (`apps/web`), where it runs inside Web Workers.

Everything that decides what to do (classification, budget allocation, quality search, resolution ladder, verification predicates, output naming, presets) is Rust and is shared. The pieces that differ are hosts:

| Concern | Desktop host | Web host |
|---|---|---|
| Reading inputs | `std::fs`, folders walked in Rust | `File` objects from drag-drop or pickers, read in a worker |
| Writing outputs | next to the source, atomic no-overwrite (section 3.10) | OPFS, then download or File System Access write |
| Parallelism | rayon thread pool inside the process | a pool of Web Workers, each with its own single-threaded WASM instance |
| Video and FFmpeg-only audio | FFmpeg as a child process (`cia-ffmpeg`) | WebCodecs plus mediabunny (`packages/webvideo`), planned by the same Rust planner |
| Opus encode | `opus` crate (libopus, BSD-3) | WebCodecs `AudioEncoder` |
| HEIC input | OS decoder (macOS sips, Windows WIC), else FFmpeg | Safari's native decoder only |

## 2.3 Monorepo layout

One private Git repository, `Hunter-Boone/CompressItAll`, rooted at `/mnt/nas/SharedFolder2/projects/CompressItAll`. Agents MUST work in a local clone (`/home/hunter/work/CompressItAll`) and push, never edit in the NAS working tree; ConvertSave lost eight days to uncommitted edits sitting in its NAS checkout.

```
CompressItAll/
  Cargo.toml                    # Rust workspace (members: crates/*, apps/desktop/src-tauri, xtask)
  Cargo.lock
  rust-toolchain.toml           # stable, pinned (e.g. 1.90.0), targets: wasm32-unknown-unknown
  deny.toml                     # cargo-deny: license allowlist, bans, advisories
  package.json                  # npm workspaces: apps/*, packages/*
  package-lock.json
  .github/workflows/            # test.yml, release-desktop.yml, promote.yml, web.yml, site.yml, nightly-matrix.yml
  .cargo/config.toml            # wasm32 CFLAGS and linker settings (section 2.5)
  docs/
    DESIGN.md                   # this file
    DECISIONS.md                # one line per non-obvious implementation decision
    presets-sources.md          # generated from presets.json: every limit with its source URL and check date
  crates/
    cia-core/                   # job model, presets, budgets, search, naming, verification predicates, logging. No codecs, no unsafe.
    cia-image/                  # image planner and codecs (mozjpeg, libwebp, oxipng, ravif, gif, quantette)
    cia-mozjpeg/                # our thin safe wrapper over mozjpeg-sys (encode to memory); builds on wasm
    cia-webp/                   # our thin safe wrapper over libwebp-sys (lossy, lossless, alpha); builds on wasm
    cia-pdf/                    # PDF optimizer on lopdf
    cia-office/                 # OOXML / ODF / EPUB repack
    cia-archive/                # zip, 7z, tar.zst, tar.xz writers; zip/7z/tar/gz readers
    cia-audio/                  # Symphonia decode, FLAC/WAV encode, Opus encode (native), Ogg mux, resampling
    cia-video-plan/             # pure video budget math and ladder, shared by FFmpeg runner and WebCodecs pipeline
    cia-ffmpeg/                 # native only: FFmpeg install/verify, ffprobe, encoder detection, command builder, runner
    cia-license/                # token verification (Ed25519), product key checksum, device id (native feature)
    cia-engine/                 # orchestrator: type detection, dispatch, job runner, events, cancellation, output sink trait
    cia-wasm-libc/              # minimal libc shim so mozjpeg/libwebp/zstd/libdeflate link on wasm32-unknown-unknown
    cia-wasm/                   # wasm-bindgen exports for the web host
    cia-cli/                    # `cia` binary: compress, inspect, plan, verify, matrix, presets
  xtask/                        # cargo xtask: fixtures, wasm, licenses, presets-doc
  apps/
    desktop/
      index.html
      src/main.tsx              # mounts @cia/ui with the Tauri EngineHost
      src-tauri/                # crate `cia-desktop`: commands, events, settings, license, updater, clipboard, drag-out
        tauri.conf.json
        capabilities/default.json
        Info.plist
    web/
      index.html
      src/main.tsx              # mounts @cia/ui with the WASM EngineHost
      src/workers/engine.worker.ts
      src/workers/video.worker.ts
      src/sw.ts                 # service worker (offline shell + wasm cache)
      public/_headers           # COOP/COEP/CSP for Cloudflare Pages
    site/                       # Next.js 15: marketing, /buy, /success, /account, /api/v1/*
  packages/
    brand/                      # brand.json (name, colours, URLs), logo SVGs, app icons source
    ui/                         # all React screens and components shared by desktop and web
    tokens/                     # tokens.ts + index.css variables + tailwind preset (ConvertSave approach)
    presets/                    # presets.json (single source of truth) + generated TS types
    engine-client/              # EngineHost interface, Tauri adapter, WASM-worker adapter, event types
    webvideo/                   # WebCodecs + mediabunny video/audio pipeline for the web host
    api-types/                  # zod schemas for every /api/v1 request and response
    licenses-data/              # generated third-party notices consumed by the Licenses page
  fixtures/
    README.md                   # where every real-world fixture came from
    synth/                      # generated by `cargo xtask fixtures` (gitignored)
    real/                       # real-world samples (gitignored; canonical copy on the NAS, see 7.2)
  e2e/
    web/                        # Playwright against the real WASM build
    native/                     # WebdriverIO + tauri-driver against the built desktop binary
  tools/lab/                    # thin wrappers around ConvertSave/TestLab scripts for this repo
```

`apps/site` lives in the same repo so the license token format, product key checksum and API schemas are shared code rather than copied code. Vercel builds it with root directory `apps/site`.

## 2.4 What is Rust and what is TypeScript

Rust owns: type detection (magic bytes via the `infer` crate plus our own checks for OOXML/ODF/EPUB), every planner, every encoder call, every verification predicate, presets parsing and validation, output naming, license token verification, product key validation, job logs.

TypeScript owns: all UI, the EngineHost adapters, the web worker pool, the WebCodecs video pipeline (encode and mux happen in browser APIs; the Rust planner decides bitrates and resolutions and verifies the result), OPFS and download handling, the site and its API.

The TypeScript EngineHost interface (`packages/engine-client/src/host.ts`):

```ts
export interface EngineHost {
  readonly kind: "desktop" | "web";
  capabilities(): Promise<Capabilities>;
  addInputs(inputs: InputSource[]): Promise<InputItem[]>;       // desktop: paths; web: File/FileSystemHandle
  removeInput(itemId: string): void;
  preview(req: PlanRequest): Promise<PlanPreview>;              // the prediction shown before Compress
  run(req: PlanRequest, onEvent: (e: EngineEvent) => void): Promise<JobHandle>;
  cancel(jobId: string): Promise<void>;
  actions: PlatformActions;                                      // reveal, copy, open, drag-out, save
}

export interface PlatformActions {
  revealInFolder?(artifactId: string): Promise<void>;           // desktop only
  copyFilesToClipboard?(artifactIds: string[]): Promise<void>;  // desktop only
  openFile?(artifactId: string): Promise<void>;                 // desktop only
  startDragOut?(artifactIds: string[]): Promise<void>;          // desktop only
  download?(artifactIds: string[]): Promise<void>;              // web only
  saveToFolder?(artifactIds: string[]): Promise<void>;          // web, Chromium only
}
```

`Capabilities`, `PlanRequest`, `PlanPreview`, `EngineEvent`, `InputItem` and every other shape crossing the boundary are Rust structs with `#[derive(Serialize, Deserialize, ts_rs::TS)]` in `cia-core`; `cargo xtask types` writes them to `packages/engine-client/src/generated/`. CI fails if the generated files differ from what is committed.

## 2.5 WASM build plan

**Target and tooling.** `wasm32-unknown-unknown`, wasm-bindgen 0.2.x with `--target web`. Build with `cargo xtask wasm`, which runs `cargo build -p cia-wasm --release --target wasm32-unknown-unknown`, then `wasm-bindgen-cli` (pinned to the exact wasm-bindgen crate version in Cargo.lock) and `wasm-opt -O3 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-simd` (binaryen). Do not use wasm-pack; calling wasm-bindgen-cli directly removes a tool and avoids version drift between the CLI and the crate.

**C code on wasm.** Several codecs are C. A spike on this VM (2026-10-02, clang 14 plus the wasi-sdk 25 sysroot) showed these compile for `wasm32-unknown-unknown` when built with:

```toml
# .cargo/config.toml
[env]
CFLAGS_wasm32_unknown_unknown = "--sysroot=/opt/wasi-sdk/share/wasi-sysroot -D__wasi__ -D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS -D_WASI_EMULATED_MMAN"
CC_wasm32_unknown_unknown = "clang"
AR_wasm32_unknown_unknown = "llvm-ar"
```

Compiled OK together in one crate: image 0.25, oxipng 9 (with libdeflate C), zopfli 0.8, zstd 0.13 (zstd-sys C), libwebp-sys (lossy and lossless libwebp C), ravif 0.13 (pure Rust rav1e), lopdf 0.36 (needs getrandom's `wasm_js` feature), flate2 with `rust_backend`, lzma-rs, sevenz-rust2 0.19, symphonia 0.5 decode, jpeg-encoder 0.6, image-webp, zip 4, brotli 8, and the C part of mozjpeg-sys 2.

Failed: the safe `mozjpeg` wrapper crate (it calls `libc::free` and `fdopen`), the `opus` crate (its sys crate needs a cmake toolchain), and `mp3lame-encoder` (autoconf, and LGPL anyway).

Consequences, all MUST:

1. `crates/cia-mozjpeg` is our own wrapper over `mozjpeg-sys`. It encodes RGB/RGBA/Gray buffers to memory with `jpeg_mem_dest`, sets `JCP_MAX_COMPRESSION`, progressive scans, trellis quantisation, optimized Huffman tables, and chroma subsampling per call. It installs a custom `error_exit` that calls a Rust `extern "C"` function which panics. On native, build mozjpeg-sys with its `unwinding` feature (the C is compiled with `-fexceptions`, which makes unwinding through it defined), declare the callback `extern "C-unwind"`, and catch the panic with `catch_unwind` at the wrapper boundary; on wasm (panic=abort) it traps the instance, and the worker treats a trap as a failed attempt and re-instantiates the module (section 2.6). We never pass invalid input to the encoder, so traps should not happen in practice.
2. `crates/cia-webp` wraps `libwebp-sys` directly (the `webp` crate does not build for wasm, jaredforth/webp issue #20). It exposes lossy with quality and alpha quality, lossless with `exact` off and method 6, and near-lossless.
3. `crates/cia-wasm-libc` provides the libc symbols those C libraries need on `wasm32-unknown-unknown`: `malloc`, `calloc`, `realloc`, `free` (backed by Rust's global allocator with a size header), `memcmp`, `strlen` and friends where compiler-builtins does not, `abort` (calls `core::arch::wasm32::unreachable`), and no-op `fprintf`/`fflush`/`getenv` stubs. zstd-sys brings its own shim; ours must not define duplicate symbols (gate each symbol with a feature named after the library that needs it).
4. **Spike gate.** Milestone M1 (section 8) starts with a two-day spike that links the full `cia-wasm` module and encodes a 12 MP photo with mozjpeg and libwebp in headless Chrome, Firefox and WebKit. If linking or runtime fails and cannot be fixed in that time, fall back for the web build only: JPEG via `jpeg-encoder` (pure Rust, progressive, optimized Huffman; files run about 5 to 10 percent larger than mozjpeg at equal quality) and WebP via `@jsquash/webp` (Apache-2.0, Emscripten build of libwebp) behind the same `Codec` trait through a JS callback. Native keeps mozjpeg and libwebp either way. Record the outcome in DECISIONS.md.
5. Opus on the web uses WebCodecs `AudioEncoder` (Chrome/Edge 94+, Firefox 130+, Safari 26+). The pure-Rust `opus-rs` crate (BSD-3-Clause, lists wasm32-unknown-unknown as verified) is the planned fallback for older Safari, added only after it passes our round-trip and interop tests (its output decodes with libopus; duration, channel count and RMS level within 1 dB of the source). Until then, older Safari gets FLAC and WAV only, with a plain message.

**Bundle.** `cia_wasm_bg.wasm` is expected at 5 to 9 MB before compression (rav1e is the largest piece). Cloudflare Pages caps a single asset at 25 MiB, which leaves room. Serve with Brotli. The main thread calls `WebAssembly.compileStreaming(fetch(url))` once and posts the compiled `WebAssembly.Module` to every worker, so the module compiles once.

**Threads.** v1 ships a single-threaded WASM build and gets parallelism from a pool of workers. Reason: `wasm-bindgen-rayon` needs a pinned nightly toolchain, `-Z build-std` and atomics flags; that is a maintenance cost the first release does not need, because most jobs have several files or several candidate encodes that parallelise naturally across workers. Milestone M10 adds `cia_wasm_mt_bg.wasm` (rayon inside the module, for single large AVIF and oxipng jobs), loaded only when `self.crossOriginIsolated === true`.

**Cross-origin isolation from day one.** The web app's origin (`app.<domain>`) is static and contains no third-party content, so it is served cross-origin isolated now, and the threaded build later needs no header change. `apps/web/public/_headers`:

```
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  Cross-Origin-Resource-Policy: same-origin
  Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; connect-src 'self' https://www.<domain>; img-src 'self' blob: data:; media-src 'self' blob:; style-src 'self' 'unsafe-inline'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'
  Referrer-Policy: no-referrer
  Permissions-Policy: camera=(), microphone=(), geolocation=()
  X-Content-Type-Options: nosniff
/assets/*
  Cache-Control: public, max-age=31536000, immutable
/*.wasm
  Content-Type: application/wasm
  Cache-Control: public, max-age=31536000, immutable
```

The only network request the web app makes is to the license API on `www.<domain>` (CORS; responses carry `Access-Control-Allow-Origin: https://app.<domain>`). Payment never happens on the app origin; Paddle's overlay needs third-party frames that `require-corp` would block, which is the reason the app and the site are separate origins.

**Web Workers.** `apps/web/src/workers/engine.worker.ts` hosts one WASM instance. The pool size is `clamp(navigator.hardwareConcurrency - 1, 1, 4)`, reduced to 2 when `navigator.deviceMemory <= 4`. The pool schedules one item per worker; for a single image it runs candidate formats on separate workers. `video.worker.ts` runs the WebCodecs pipeline (WebCodecs is available in dedicated workers) and calls into a WASM instance for planning and verification.

**Large files and OPFS.** Workers read inputs straight from `File` objects (`file.slice()` streams for video through mediabunny's `BlobSource`; `arrayBuffer()` for images, PDFs and documents up to 1 GiB). Outputs are written to OPFS with `createSyncAccessHandle()` (Chrome 102+, Firefox 111+, Safari 15.2+) under `/jobs/<job_id>/`. The download is created from the OPFS `File` returned by `getFile()`, which is disk-backed, so a 900 MB output never sits in memory. OPFS job folders are deleted when the user clicks "Start over", and any older than 24 hours are deleted at app start.

**Folders.** Dropped folders are read with `DataTransferItem.webkitGetAsEntry()` in every browser. The "Choose folder" button and "Save into a folder" use `showDirectoryPicker()` where it exists (Chromium only; Firefox and Safari have no implementation). Elsewhere the folder button is hidden, and multi-file results download as one zip.

## 2.6 Failure isolation

- Desktop: each encode runs inside `std::panic::catch_unwind`; FFmpeg runs as a child process with a watchdog. A crash in one item fails that item only.
- Web: if a worker traps or runs out of memory, the pool marks the attempt as failed, terminates and replaces the worker, and continues with the next candidate. Three traps for one item fail the item with "Smidge ran out of memory on this file. Try the desktop app."

## 2.7 Feature matrix

"FFmpeg" means the LGPL build the user downloads in setup (section 4.8). "Chromium" is Chrome or Edge 120+. Safari means 26+ unless noted.

| Feature | Desktop, no FFmpeg | Desktop + FFmpeg | Web, Chromium | Web, Firefox 130+ | Web, Safari 26 |
|---|---|---|---|---|---|
| JPEG, PNG, WebP, GIF, BMP, TIFF input | yes | yes | yes | yes | yes |
| AVIF input | no | yes (FFmpeg decode) | yes | yes | yes |
| HEIC/HEIF input | macOS yes; Windows if the HEIF extension is installed; Linux no | yes | no | no | yes (browser decoder) |
| Output JPEG (mozjpeg), PNG (oxipng), WebP, AVIF, GIF | yes | yes | yes | yes | yes |
| PDF optimise | yes | yes | yes | yes | yes |
| docx/pptx/xlsx/odt/odp/epub media recompress | yes, images only | yes, images and embedded video | images only | images only | images only |
| zip / 7z / tar.zst / tar.xz packaging | yes | yes | yes | yes | yes |
| WAV/FLAC/AIFF/OGG Vorbis/MP3 input | yes | yes | yes | yes | yes |
| AAC/M4A/ALAC input | no | yes | where WebCodecs decodes it | where WebCodecs decodes it | yes |
| Output FLAC, WAV | yes | yes | yes | yes | yes |
| Output Opus (.ogg) | yes (libopus) | yes | yes | yes | yes (Safari 26+) |
| Output MP3 | no | yes (LAME in FFmpeg) | no | no | no |
| Output AAC (.m4a) | no | yes (FFmpeg native aac) | yes, except desktop Linux | no | yes |
| Video input MP4/MOV/WebM/MKV | no | yes | yes, if WebCodecs decodes the codec | same | same |
| Video input AVI/WMV/FLV/MPEG-PS/ProRes | no | yes | no | no | no |
| Video output H.264 MP4 | no | yes, via platform or GPU encoder (3.5.6) | yes | yes | yes |
| Video output VP9 WebM | no | yes (libvpx two-pass) | yes | yes | yes |
| HDR to SDR tone-mapping | no | yes (zscale + tonemap) | yes, verified browsers only | no, refused | yes, verified browsers only |
| Copy file to clipboard | yes | yes | no | no | no |
| Drag result out of the window | yes | yes | no | no | no |
| Save next to the original | yes | yes | no (download) | no (download) | no (download) |
| Choose an output folder | yes | yes | yes | no (zip download) | no (zip download) |
| Works offline | yes | yes | yes, after first visit (PWA) | same | same |

What the web app therefore cannot produce: MP3 (no permissive encoder we can ship; LAME is LGPL and the one new pure-Rust MP3 encoder on crates.io has untraceable provenance), AAC in Firefox and in Chrome on desktop Linux, HEVC output anywhere, and any video from containers or codecs WebCodecs and mediabunny cannot read. The app says so in plain words at the moment it matters (section 4.10).

---
# 3. The engine

## 3.1 Job model

A job moves through four objects: **Job** (what the user asked for), **Plan** (what the engine intends to do, shown as the prediction), **Attempts** (each encode tried), and **Result** (verified outcomes). These live in `crates/cia-core/src/model.rs`.

```rust
pub struct Job {
    pub id: JobId,                       // ULID string
    pub items: Vec<InputItem>,
    pub goal: Goal,
    pub packaging: Packaging,
    pub options: JobOptions,
    pub created_at: Timestamp,
}

pub enum Goal {
    Fit { preset_id: String, limit: ResolvedLimit },  // presets and Custom both resolve to this
    Smaller { level: SmallerLevel },                   // no target
}

pub struct ResolvedLimit {
    pub hard_bytes: u64,        // ceiling in raw output bytes (or total for per-message); output MUST be <= this.
                                // Derived in 3.11 from the preset limit, its encoding overhead and safety_bytes.
                                // Planners apply their own extra margins (video 3.5.2, audio 3.6).
    pub scope: LimitScope,      // PerFile | PerMessage
    pub max_files_per_message: Option<u32>,
    pub by_kind: BTreeMap<Kind, ResolvedKindLimit>,   // e.g. WhatsApp documents 2 GB, video 64 MB
}

pub enum SmallerLevel { KeepQuality, Smallest }   // UI: "Keep quality" (default) / "Smallest files"
pub enum Packaging { Auto, SeparateFiles, Zip, SevenZip, TarZst, TarXz }

pub struct InputItem {
    pub id: ItemId,
    pub source: SourceRef,      // desktop: absolute path; web: worker-side handle id
    pub rel_path: String,       // path relative to the dropped folder, or just the file name
    pub bytes: u64,
    pub kind: Kind,             // detected from content, not extension
    pub detail: KindDetail,     // format, dimensions, duration, streams, page count, ... filled by inspect
}

pub enum Kind { Image, AnimatedImage, Video, Audio, Pdf, OfficeDoc, Archive, Text, Other }

pub struct JobOptions {
    pub keep_photo_details: bool,     // EXIF minus GPS; default false
    pub keep_location: bool,          // GPS; default false; only meaningful with keep_photo_details
    pub allow_format_change: bool,    // default true for Fit, false for Smaller
    pub max_long_edge: Option<u32>,   // Advanced: "Limit photo size"
    pub video: VideoOptions,          // codec preference, faster mode, audio track choice, keep audio
    pub audio: AudioOptions,          // preferred output format
    pub optimise_inside_archives: bool, // default true
    pub output_dir: Option<OutputDir>,  // desktop: SameAsSource (default) | Folder(path)
}
```

Plan, attempts and results:

```rust
pub struct Plan {
    pub job_id: JobId,
    pub items: Vec<ItemPlan>,
    pub packaging: PackagingPlan,       // resolved Auto, archive name, predicted overhead
    pub predicted_total_bytes: u64,
    pub verdict: PlanVerdict,
}

pub struct ItemPlan {
    pub item_id: ItemId,
    pub budget_bytes: Option<u64>,      // None in Smaller mode
    pub strategy: Strategy,             // per-type enum with the chosen candidate list / ladder rung
    pub prediction: Prediction,         // user-facing numbers: size, dims, fps, quality label, notes
}

pub enum PlanVerdict {
    WillFit { quality: QualityLabel },                 // Great | Good | Okay
    Uncertain { quality: QualityLabel },               // video on a hardware encoder; shown as "about"
    CannotFit { refusal: Refusal },                    // shown before any encoding happens
}

pub struct Attempt {
    pub n: u32,
    pub item_id: ItemId,
    pub encoder: String,                // "mozjpeg", "libwebp-lossless", "h264_nvenc", "libvpx-vp9-2pass" ...
    pub params: serde_json::Value,      // quality, dims, bitrate, preset, crf, ...
    pub output_bytes: Option<u64>,
    pub score: Option<f32>,             // SSIMULACRA2 where computed
    pub elapsed_ms: u64,
    pub verdict: AttemptVerdict,        // Fits | Over { by_bytes } | BelowFloor | Error { message } | Cancelled
}

pub enum ItemOutcome {
    Fitted(Artifact),
    KeptOriginal(Artifact),     // the original already fits and nothing we tried was meaningfully smaller
    Refused(Refusal),
    Failed(Failure),            // unexpected error: damaged input, encoder crash after retries
    Cancelled,
}

pub struct Artifact {
    pub id: ArtifactId,
    pub location: OutputLocation,   // desktop path, or OPFS path on web
    pub file_name: String,
    pub bytes: u64,
    pub format: OutputFormat,
    pub summary: String,            // "720p, 30 fps, H.264", "4032 × 3024 JPEG", "12 pages"
    pub quality: QualityLabel,
    pub verification: VerificationReport,
}

pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,                // final user-facing sentence, built by cia-core/src/copy.rs
    pub smallest_bytes: Option<u64>,    // best we reached, for the explanation only
    pub suggestions: Vec<Suggestion>,
}

pub enum RefusalCode {
    TooLongForLimit { max_duration_ms: u64 },
    BelowQualityFloor,
    CannotShrinkType,
    TooManyFilesForMessage { max: u32 },
    TotalTooBig,
    NeedsFfmpeg,
    NoEncoder { codec: String },        // desktop: no usable encoder; web: the browser lacks it
    UnsupportedInput { what: String },
    Encrypted,
}

pub enum Suggestion {
    Trim { max_duration_ms: u64 },
    PickPreset { preset_id: String, predicted_bytes: u64 },
    SplitIntoMessages { groups: Vec<Vec<ItemId>> },
    RemoveFiles { item_ids: Vec<ItemId> },
    InstallFfmpeg,
    UseDesktopApp,
    UseOtherBrowser { browser: String },
}
```

`JobSummary` (the Result) is `{ job_id, outcomes: Vec<(ItemId, ItemOutcome)>, packaged: Option<Artifact>, total_bytes, verdict: JobVerdict }` with `JobVerdict = AllFit | SomeFit | NoneFit | Cancelled`. For a PerMessage limit the job is all-or-nothing: if the total does not fit, nothing is written and the verdict is `NoneFit` with suggestions.

## 3.2 State machines

Job:

```
Created --inspect--> Inspecting --ok--> Ready --preview--> Planned --run--> Running --> Verifying --> Done
   |                     |                                    |              |             |
   |                     +--all inputs unreadable--> Done(NoneFit)           |             +--> Done(AllFit | SomeFit | NoneFit)
   +------------------------------------- cancel (any state before Done) --------------------> Done(Cancelled)
```

- `Ready` means inputs are inspected but no goal is picked yet. Changing the goal or the files returns to `Ready` and invalidates the plan.
- `Planned` holds the preview. The Compress button is enabled only in `Planned` with a verdict other than `CannotFit`.

Item:

```
Queued -> Inspecting -> Planning -> Encoding(n) -> Verifying(n) -+-> Fitted
                                        ^                        +-> KeptOriginal
                                        |                        +-> Retry -> Encoding(n+1)   (n+1 <= max_attempts)
                                        +------------------------+-> Refused   (no candidate fits above the floor)
                                                                 +-> Failed    (errors exhausted the encoder chain)
any state -> Cancelled
```

Every transition emits an `EngineEvent` (section 3.12). Attempts are capped per type: images 24 encodes in total across candidates, PDFs 8 full rewrites, documents 6, audio 4, video 3 on software encoders and 4 on hardware encoders.

## 3.3 Type detection and dispatch

`cia-engine/src/detect.rs` reads the first 64 KiB and decides `Kind` from content. File extensions are only a tiebreaker.

| Content | Kind | Planner |
|---|---|---|
| JPEG, PNG, WebP (still), GIF (1 frame), BMP, TIFF, AVIF, HEIC/HEIF, JPEG XL | Image | 3.4 |
| GIF (>1 frame), APNG, animated WebP | AnimatedImage | 3.4.8 |
| MP4/MOV/M4V, MKV/WebM, AVI, WMV/ASF, FLV, MPEG-TS/PS, 3GP | Video | 3.5 |
| MP3, WAV, FLAC, OGG/Opus, M4A/AAC/ALAC, AIFF, WMA, CAF | Audio | 3.6 |
| PDF | Pdf | 3.7 |
| ZIP with `[Content_Types].xml` (docx/pptx/xlsx and macro variants), ZIP with `mimetype` = ODF or EPUB | OfficeDoc | 3.8 |
| ZIP, 7z, tar, tar.gz/bz2/xz/zst, gz | Archive | 3.9.3 |
| UTF-8/UTF-16 text, CSV, JSON, XML, SVG, logs | Text | 3.9.4 |
| anything else | Other | 3.9.4 |

## 3.4 Image planner

### 3.4.1 Decode and normalise

1. Decode with `image` (JPEG via zune-jpeg, PNG, GIF, WebP, BMP, TIFF) and JPEG XL with `jxl-oxide`. AVIF and HEIC input need a decoder we do not compile in. HEIC on desktop uses the operating system's decoder first, so iPhone photos work without any setup where the OS can read them: macOS through `sips -s format png in.heic --out tmp.png` (the ConvertSave pattern; ImageIO always has HEIC), Windows through WIC (`windows` crate, `Win32_Graphics_Imaging`) when the HEIF Image Extensions are installed, which they are on most Windows 11 PCs. Otherwise, and for AVIF, desktop uses FFmpeg (`ffmpeg -i in.heic -frames:v 1 -f image2pipe -c:v png -`; FFmpeg 7.1+ handles tiled HEIC grids, and the build has libaom and dav1d for AVIF). The web uses `createImageBitmap` (AVIF in every current browser, HEIC in Safari only). When no decoder is available, the desktop row says "Needs video support", since the FFmpeg download also opens these photos.
2. Read EXIF with `kamadak-exif`. Apply Orientation (values 2 to 8) to the pixels. The output never carries an Orientation tag other than 1, so every viewer shows it upright.
3. Colour: keep the ICC profile if it is not sRGB (Display P3 iPhone photos stay P3). Drop an sRGB profile; it adds about 3 KB and changes nothing.
4. Metadata: strip everything by default. With "Keep photo details" on, copy EXIF date, camera make/model, exposure fields with `little_exif`; GPS only when "Keep location" is also on.
5. 16-bit PNG and TIFF are reduced to 8 bits per channel unless the output stays lossless PNG in Smaller mode.

### 3.4.2 Classify

Computed on a 512 px thumbnail for speed, except alpha which uses full resolution:

- `has_alpha`: any pixel with alpha < 255.
- `unique_colours`: exact count up to 65,536 on the thumbnail.
- `flat_ratio`: share of 8×8 blocks whose luma range is at most 2.
- Class:
  - `Graphic` if `unique_colours <= 4096` or `flat_ratio >= 0.5` (screenshots, diagrams, logos, UI).
  - `Photo` otherwise.

### 3.4.3 Candidates

Candidates are tried in order, filtered by the preset's allowed formats (`formats.image`) and by `allow_format_change`. When format change is off, only the source format's candidates are used.

| Class | Alpha | Candidates (in order) |
|---|---|---|
| Photo | no | 1. mozjpeg (quality search) 2. WebP lossy (quality search) 3. AVIF (quality search, Smaller mode with "Modern formats" only) |
| Photo | yes | 1. WebP lossy with alpha 2. PNG palette-quantised RGBA (quantette, 256 colours, dithered) 3. PNG lossless (oxipng) |
| Graphic | no | 1. PNG lossless (oxipng, level 4, strip) 2. WebP lossless (method 6) 3. PNG palette-quantised (quantette 256 then 128, 64 colours) 4. mozjpeg, quality searched between 85 and 92, 4:4:4 chroma (text stays sharp) |
| Graphic | yes | 1. PNG lossless (oxipng) 2. WebP lossless 3. PNG palette-quantised RGBA 4. WebP lossy with alpha |

JPEG is never chosen for an image with alpha. "Flatten transparency onto white" is an Advanced toggle (off by default) that makes JPEG eligible.

Encoder settings that are fixed, not searched:

- mozjpeg: progressive, trellis on, optimize scans, `JCP_MAX_COMPRESSION`, chroma 4:2:0 when quality < 90 and 4:4:4 at 90 and above (and always 4:4:4 for Graphic).
- WebP lossy: method 6, `sns_strength` 50, `filter_strength` 60, `use_sharp_yuv` 1, `alpha_quality` = quality.
- WebP lossless: method 6, quality 100 (effort), `exact` 0.
- oxipng: preset 4, `strip = Safe` (we already removed metadata), `zopfli` only when the image is under 2 MP (zopfli is slow; zopfli via the `zopfli` crate).
- AVIF (ravif): speed 6, quality searched, alpha quality = quality, 10-bit off (8-bit output), threads 1 on wasm.

### 3.4.4 Quality search

For each lossy candidate, find the highest quality whose output fits the item budget. Every encode is cached by `(candidate, quality, width, height)` and never repeated.

```
q_max = { jpeg: 92, webp: 92, avif: 85 }
q_min = { jpeg: 45, webp: 45, avif: 40 }      // the quality floor; below this we downscale instead

fn search(candidate, budget) -> Option<(q, bytes)>:
    s_hi = encode(q_max)
    if s_hi < budget: return (q_max, s_hi)
    s_lo = encode(q_min)
    if s_lo >= budget: return None                      // floor does not fit at this size
    lo, hi = q_min, q_max
    for _ in 0..6:                                      // at most 8 encodes per candidate per size
        if hi - lo <= 1: break
        t = (ln(budget) - ln(s_lo)) / (ln(s_hi) - ln(s_lo))   // interpolate in log-size space
        q = clamp(round(lo + t * (hi - lo)), lo + 1, hi - 1)
        s = encode(q)
        if s < budget: lo, s_lo = q, s
        else:          hi, s_hi = q, s
    return (lo, s_lo)
```

Log-size interpolation converges in 3 to 4 encodes for typical photos, because JPEG and WebP size grows roughly exponentially with quality.

### 3.4.5 Choosing among candidates

1. Lossless candidates that fit win immediately (they are exact).
2. If two or more lossy candidates fit, compute SSIMULACRA2 (`ssimulacra2` crate, BSD-2-Clause) between the source and each result, both downscaled to at most 1 MP with the same filter. Pick the highest score. Ties within 1.0 point go to the more compatible format in this order: JPEG, PNG, WebP, AVIF.
3. Candidates run in parallel (rayon on desktop, separate workers on web), at most 3 at a time.

Do not use DSSIM: the `dssim` crate is AGPL-3.0 and is forbidden (section 10).

### 3.4.6 Downscale, only as a last resort

If no candidate fits at the quality floor at full size:

```
best_floor = min over lossy candidates of size at q_min
scale = sqrt(budget / best_floor) * 0.97
new_long_edge = floor(long_edge * scale), keep aspect ratio, round both dimensions to integers >= 1
if new_long_edge < 480: return Refused(BelowQualityFloor)
resize with fast_image_resize, Lanczos3, in linear light for Photo and sRGB for Graphic
re-run 3.4.4 for every lossy candidate at the new size (at most 3 downscale rounds)
```

Never upscale. `max_long_edge` from Advanced is applied before the search, not as a fallback.

### 3.4.7 Smaller mode for images

No budget. The search target is a quality score instead of a size:

- KeepQuality: the lowest quality whose SSIMULACRA2 score is at least 85 ("visually lossless" range), searched in q 60 to 92 for JPEG and WebP, with the same log interpolation on score.
- Smallest: score at least 70, q 45 to 85.
- Lossless candidates (oxipng, WebP lossless) are always tried for Graphic.
- If the best result is not at least 5 percent smaller than the original, the outcome is `KeptOriginal` and the row says "Already as small as it gets." The original bytes are copied (or referenced, see 3.10), never re-encoded for nothing.

In Fit mode, when the original already fits, only the cheap lossless step runs (oxipng for PNG; JPEG is left alone). If that is not 5 percent smaller, the original is kept.

### 3.4.8 Animated images

GIF, APNG, animated WebP. (gifsicle is GPL and is not used.)

1. Decode frames with `gif` / `image`, coalesce to full frames.
2. Candidates by allowed formats: GIF re-encode (per-frame palette via quantette, unchanged-pixel transparency between frames, LZW from the `gif` crate), animated WebP lossy (libwebp `WebPAnimEncoder`), and MP4/WebM via the video planner when the preset allows video and the destination plays it inline (Discord does; email does not).
3. Fit steps, in order, each tried before the next: reduce colours (256, 128, 64); drop every second frame and double the delay of the kept frames (only when the frame rate is above 15 fps); downscale by the 3.4.6 formula; refuse below 240 px long edge.

## 3.5 Video planner

These are the rules from `video-compressor-build-prompt.md`, adapted to the encoders a commercially redistributable stack can use. `cia-video-plan` holds the math; `cia-ffmpeg` (desktop) and `packages/webvideo` (web) execute it.

### 3.5.1 Probe

Desktop: `ffprobe -v error -print_format json -show_format -show_streams -show_frames -read_intervals %+5 -select_streams v:0 <file>` plus a second call without `-show_frames` for all streams. Web: mediabunny `Input` (`getPrimaryVideoTrack`, `getAudioTracks`, `computeDuration`, packet stats over the first 5 s).

`VideoProbe` fields: duration_ms, container, video codec, coded width/height, display width/height after rotation (from the display matrix / `rotation` side data), rotation degrees, avg and max frame rate, `is_vfr` (more than 2 percent spread in packet durations over the first 5 s), pixel format, colour transfer and primaries, `is_hdr` (transfer `smpte2084` or `arib-std-b67`), Dolby Vision profile if present, audio streams with codec, channels, sample rate, bitrate, and `title` tags.

A probe failure fails the item: "This video file is damaged or incomplete, so Smidge can't read it."

### 3.5.2 Budget

```
overhead_bytes = container_overhead(container, duration_s, fps, audio_tracks_out)
    mp4:  4096 + ceil(duration_s) * (fps * 14 + audio_tracks_out * 47 * 10)
    webm: 4096 + ceil(duration_s) * (fps * 12 + audio_tracks_out * 50 * 8)
margin = 0.96 for two-pass software encoders, 0.92 for single-pass hardware / WebCodecs encoders
budget_bits = (hard_bytes - overhead_bytes) * 8 * margin
total_bps = budget_bits / duration_s
```

### 3.5.3 Audio first

Output audio codec follows the container: AAC (FFmpeg's native `aac` encoder, or WebCodecs `mp4a.40.2`) in MP4, Opus in WebM. Ladder, highest first:

- AAC: 128 kb/s stereo, 96 stereo, 64 stereo, 48 mono, 32 mono
- Opus: 96 kb/s stereo, 64 stereo, 48 stereo, 32 mono, 24 mono

Pick the highest rung whose bitrate is at most 15 percent of `total_bps`. If even the lowest rung exceeds 15 percent, use the lowest rung. Source audio that is already mono never upmixes. No audio stream, or "Remove sound" in Advanced, sets audio_bps to 0.

Multiple audio tracks (OBS): default mixes all tracks into one with `amix=inputs=N:duration=longest:normalize=0` followed by `alimiter=limit=0.97` to prevent clipping. Advanced offers "Sound: mix all tracks / track 1 / track 2 ...", listing track titles when tagged.

### 3.5.4 Resolution and frame-rate ladder

```
video_bps = total_bps - audio_bps
rungs = []
start = (display_w, display_h, min(src_fps, preset.max_fps, 60))
rungs.push(start)
if start.fps > 30: rungs.push(same size, fps 30)         // 60 -> 30 first
for h in [1080, 720, 540, 480, 360] where h < display_short_edge_height:   // never upscale
    rungs.push(scaled to height h keeping aspect, fps min(start.fps, 30))
apply preset.max_height (WhatsApp 720) by dropping rungs above it

floor_bpp(codec, h):  h264: h >= 720 -> 0.050, h >= 480 -> 0.060, else 0.070
                      vp9:  0.75 * h264 value;   av1: 0.60 * h264 value
for rung in rungs:
    bpp = video_bps / (rung.w * rung.h * rung.fps)
    if bpp >= floor_bpp(codec, rung.h): choose rung; break
if none: refuse (3.5.5)
quality label: bpp >= 2.2 * floor -> Great; >= 1.4 * floor -> Good; else Okay
```

"Height" means the short edge for portrait video, so a 1080×1920 phone video steps 1080, 720, 540, 480, 360 on its short side. Dimensions are rounded to even numbers (`scale=-2:H` style), which yuv420p requires.

Worked example, which MUST exist as a unit test in `cia-video-plan`: Jayden's 70 s 1440p60 clip, Discord Free, NVENC (margin 0.92). `hard_bytes` = 20,971,520 - 65,536 - 1 = 20,905,983. Overhead = 4096 + 70 × (30 × 14 + 1 × 470) = 66,396 bytes. `budget_bits` = (20,905,983 - 66,396) × 8 × 0.92 = 153,379,360; `total_bps` = 2,191,134. Audio: 15 percent is 328,670, so AAC 128 kb/s stereo. `video_bps` = 2,063,134. Rungs: 1440p60 bpp 0.0093, 1440p30 0.019, 1080p30 0.033, 720p30 0.0746, which is at least 0.050, so 720p30 is chosen; 0.0746 / 0.050 = 1.49, label Good. Predicted size = 2,191,134 × 70 / 8 + 66,396 = 19,238,816 bytes, shown as "about 19.2 MB". The same clip at 4 min 54 s would drop to the 360p floor; one second longer is refused.

If the source already fits the hard limit and its codecs are allowed by the preset, the outcome is `KeptOriginal` (copy). If it fits but the container is wrong (H.264+AAC in MKV for WhatsApp), remux with `-c copy -movflags +faststart` and verify.

### 3.5.5 Refuse before encoding

If no rung meets the floor, do not encode. Compute

```
min_bps = floor_bpp(codec, 360) * w360 * h360 * 30 + lowest_audio_rung_bps
max_duration_s = floor(((hard_bytes - overhead_at_that_duration) * 8 * margin) / min_bps)
```

(solve by iterating twice, since overhead depends on duration). The Refusal carries `TooLongForLimit { max_duration_ms }`, a `Trim` suggestion, and `PickPreset` suggestions for every other preset in which the full video would fit at Okay or better, with predicted sizes. UI copy: "Too long to fit in 20 MB at watchable quality. Trim it to under 4 min 50 s, or choose Discord Nitro Basic (50 MB)."

### 3.5.6 Encoder selection

The FFmpeg build Smidge downloads is LGPL-only and contains no software H.264 encoder (no x264). H.264 output therefore comes from encoders whose patent licensing is the platform's or the GPU vendor's business, exposed through FFmpeg. Chain for an H.264 MP4 target, first available and working wins:

1. `libx264`, only if the user pointed Smidge at their own FFmpeg that has it (Settings → Video support → "Use a different FFmpeg"). Two-pass.
2. macOS: `h264_videotoolbox`.
3. NVIDIA: `h264_nvenc`. AMD: `h264_amf`. Intel: `h264_qsv`. Linux with VA-API: `h264_vaapi`.
4. Windows: `h264_mf` (Media Foundation, present on every Windows 10/11).
5. Nothing left: if the preset allows WebM, switch the target to VP9 WebM (`libvpx-vp9`, two-pass). Otherwise refuse with `NoEncoder { codec: "h264" }`: "This computer has no H.264 video encoder that Smidge can use, and WhatsApp needs H.264. Smidge can make a WebM file for Discord instead." (Only plausible on Linux without a GPU.)

Encoder detection is the ConvertSave `builder_options.rs` idea: parse `ffmpeg -hide_banner -encoders`, then run a 1-second test encode (`-f lavfi -i testsrc2=s=640x360:r=30 -t 1`) per candidate at first use and cache the result in `tools/ffmpeg/<version>/encoders.json`, because a listed hardware encoder can still fail when the driver or GPU is missing.

"Faster (uses your graphics card)" in Advanced is on by default. When off, the chain skips GPU encoders but keeps VideoToolbox and Media Foundation, since those are the only H.264 encoders on many machines.

### 3.5.7 Encode commands

Common input and filter chain:

```
ffmpeg -hide_banner -nostdin -y -progress pipe:1 -nostats
  -i INPUT
  [-ss START -to END]                        # trim, placed after -i for frame accuracy
  -map 0:v:0 [-map 0:a? per audio option]
  -filter_complex "<video chain>[;<audio chain>]"
  -fps_mode cfr -r FPS                       # constant frame rate output, fixes VFR phone video
  -pix_fmt yuv420p
  -map_metadata -1 -map_chapters -1
  -movflags +faststart                       # MP4 only
  OUTPUT.partial.mp4
```

Video chain:

- SDR: `scale=W:H:flags=lanczos,setsar=1`
- HDR (PQ or HLG, including iPhone Dolby Vision profile 8.4, which carries an HLG base layer): `zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p,scale=W:H:flags=lanczos,setsar=1`, and tag the output `-color_primaries bt709 -color_trc bt709 -colorspace bt709`.
- Rotation: FFmpeg applies the display matrix automatically during decode (autorotate is on by default), and the output has no rotation side data. W and H are computed from display dimensions.

Audio chain: `aresample=async=1:first_pts=0` (keeps sync with VFR sources), then `amix` for multiple tracks, then `-c:a aac -b:a RATE -ac CH` or `-c:a libopus -b:a RATE -ac CH`.

Per encoder:

| Encoder | Rate control arguments |
|---|---|
| libvpx-vp9 two-pass | pass 1: `-c:v libvpx-vp9 -b:v BR -minrate BR*0.5 -maxrate BR*1.45 -row-mt 1 -tile-columns 2 -deadline good -cpu-used 4 -pass 1 -an -f null`; pass 2: same with `-cpu-used 2 -pass 2` and audio |
| libx264 two-pass (user's FFmpeg) | `-c:v libx264 -preset medium -profile:v high -b:v BR -maxrate BR*1.5 -bufsize BR*2 -pass 1/2` |
| h264_videotoolbox | `-c:v h264_videotoolbox -profile:v high -b:v BR -maxrate BR*1.2 -bufsize BR*2 -allow_sw 1 -realtime 0` |
| h264_nvenc | `-c:v h264_nvenc -preset p6 -tune hq -multipass fullres -rc vbr -b:v BR -maxrate BR*1.3 -bufsize BR*2 -profile:v high` |
| h264_amf | `-c:v h264_amf -quality quality -rc vbr_peak -b:v BR -maxrate BR*1.3 -bufsize BR*2 -profile:v high` |
| h264_qsv | `-c:v h264_qsv -preset slower -b:v BR -maxrate BR*1.3 -bufsize BR*2 -profile:v high` |
| h264_vaapi | `-vaapi_device /dev/dri/renderD128` with `format=nv12,hwupload` appended to the chain, `-c:v h264_vaapi -rc_mode VBR -b:v BR -maxrate BR*1.3` |
| h264_mf | `-c:v h264_mf -rate_control pc_vbr -b:v BR -scenario display_remoting -hw_encoding 1` (fall back to `-hw_encoding 0` on error) |

### 3.5.8 Retry on overshoot, and undershoot

```
after each encode:
    actual = file size
    if actual <= hard_bytes and verify passes: done
    if attempt == max_attempts: Failed("Smidge couldn't get this video under the limit. The closest was 21.3 MB.")
    factor = (hard_bytes / actual) * (0.97 software | 0.93 hardware)
    video_bps = video_bps * factor
    if video_bps < floor at current rung: move down one rung (3.5.4), recompute
    re-encode
```

An undershoot (output below 85 percent of the budget) is accepted as is. Re-encoding to fill the budget doubles the wait for a gain few people can see.

A hardware encoder that exits non-zero or produces a file that fails verification is marked broken for this session and the next encoder in the chain is used for the retry. Every attempt goes into the job log.

### 3.5.9 Watchdog and cancel

FFmpeg's `-progress pipe:1` output is read line by line; `out_time_us` drives progress and time remaining. No progress line for 60 s kills the process ("The video encoder stopped responding."). Cancel kills the process tree (Windows: job object; macOS/Linux: process group) and deletes partial files. Child processes are created with `CREATE_NO_WINDOW` on Windows, and paths are passed as arguments, never through a shell, which keeps spaces, emoji and non-Latin names safe.

### 3.5.10 Web video pipeline

`packages/webvideo/src/transcode.ts` uses mediabunny (MPL-2.0) for demux and mux and WebCodecs for decode and encode.

1. Capability check before planning: `VideoDecoder.isConfigSupported` for the source track's codec string, `VideoEncoder.isConfigSupported` for `avc1.640028` (H.264 High 4.0) at the planned size and bitrate with `hardwareAcceleration: "no-preference"`, then `vp09.00.40.08` as the WebM fallback; `AudioEncoder.isConfigSupported` for `mp4a.40.2` and `opus`. Results go into `Capabilities.web`.
2. The Rust planner (`cia_wasm::plan_video(probe, limit, caps)`) returns the rung, bitrates and container exactly as on desktop, with margin 0.92 (single pass).
3. Transcode with mediabunny's `Conversion` (`video: { width, height, codec: "avc", bitrate, frameRate, keyFrameInterval: 2 }`, `audio: { codec: "aac" | "opus", bitrate, numberOfChannels }`, output `Mp4OutputFormat({ fastStart: "in-memory" })` for outputs up to 1 GiB, otherwise `fastStart: false` streaming to OPFS through `StreamTarget`).
4. HDR sources: frames pass through an `OffscreenCanvas` 2D draw, which tone-maps to sRGB in Chrome and Safari. This path is enabled only on browsers listed in `packages/webvideo/src/hdr-verified.ts`, a list maintained from the Playwright HDR fixture result (section 7.4). Other browsers refuse with `UseDesktopApp`.
5. Mixed multi-track audio: decode each track with `AudioDecoder`, mix in a worklet-free loop in the worker (sum and limit), re-encode.
6. Verify: re-open the output with mediabunny, check duration, dimensions, codec, audio presence, size; decode the first and last second with `VideoDecoder`.
7. Retry exactly as 3.5.8.

## 3.6 Audio planner

Decode: Symphonia (MPL-2.0) with features `mp3`, `flac`, `vorbis`, `ogg`, `wav`, `pcm`, `aiff`, `caf`, `mkv`. The `aac`, `alac` and `isomp4` features stay off, the same codec posture as ConvertSave; AAC/M4A/ALAC input goes through FFmpeg on desktop and WebCodecs `AudioDecoder` on the web. Opus input decodes with libopus natively (`opus` crate) and WebCodecs on the web, since Symphonia has no Opus decoder.

Output format: the first format in the preset's `formats.audio` list that this host can produce (feature matrix 2.7), unless the user picked one in Advanced.

| Format | Encoder | Bitrate ladder (target picks the highest that fits) | Floor |
|---|---|---|---|
| MP3 | FFmpeg `libmp3lame` CBR | 192, 160, 128, 112, 96, 80, 64 kb/s; mono from 80 down; 44.1 kHz, 32 kHz at 64 | 64 kb/s mono |
| AAC (.m4a) | FFmpeg `aac` / WebCodecs | 160, 128, 96, 80, 64, 48 mono, 32 mono | 32 kb/s mono |
| Opus (.ogg) | libopus / WebCodecs | 128, 96, 64, 48, 32 mono, 24 mono, 16 mono (VOIP application below 32) | 16 kb/s mono |
| FLAC | flacenc (pure Rust) | lossless, compression level 8 | n/a |
| WAV | hound | lossless; only when nothing smaller is allowed | n/a |

Fit algorithm:

```
budget_bits = (hard_bytes - container_overhead) * 8 * 0.98
target_bps = budget_bits / duration_s
for format in allowed_formats:
    rung = highest ladder bitrate <= target_bps
    if rung exists: plan (format, rung); break
    if format is lossless and lossless size estimate (encode first 10 s, extrapolate) fits: plan lossless; break
if no plan: refuse TooLongForLimit with max_duration = floor(budget_bits / floor_bps)
encode, verify, retry with bitrate * (budget / actual) * 0.97, at most 4 attempts
```

Smaller mode for audio: WAV and AIFF become FLAC (lossless, roughly half the size); FLAC is re-encoded at level 8 and kept only if 3 percent smaller; lossy inputs are left alone (`KeptOriginal`) unless "Smallest files" is chosen, which re-encodes to Opus 96 kb/s stereo, or 48 kb/s for speech-like mono sources.

Verification: decode the whole output (native: Symphonia for MP3/FLAC/WAV, libopus for Opus, ffprobe plus `ffmpeg -v error -i out -f null -` for AAC; web: WebCodecs `AudioDecoder`), duration within 0.1 s of source or trim range, channel count as planned, size below the limit.

## 3.7 PDF planner

`cia-pdf` is built on lopdf 0.45 (MIT), which builds for wasm with the `wasm_js` feature and default `rayon` off, and which writes object streams and cross-reference streams (`Document::compress`, `save_modern`/`use_object_streams(true)`, `compression_level(9)`). pdf-rs was considered and not chosen: lopdf can both read and write, and one parser is enough.

Refuse up front:

- Encrypted PDFs, including owner-password-only ones: `Encrypted`, "This PDF is password-protected, so Smidge can't change it."
- PDFs that fail to load: `Failed`, "This PDF is damaged."

Steps:

1. **Lossless pass (always).** Remove unreferenced objects (`prune_objects`), delete page thumbnails (`/Thumb`), `/PieceInfo` and embedded XMP metadata (unless "Keep document details" is on), merge byte-identical streams (hash every stream; replace references to duplicates), re-deflate every Flate stream at level 9 (zopfli for streams under 256 KB), and save with object streams and xref streams. Keep fonts, forms, links, bookmarks and annotations untouched.
2. If the lossless result fits, or in Smaller mode with KeepQuality, stop there unless image recompression saves more than 10 percent (measured in step 3 at quality 85).
3. **Image pass.** Collect image XObjects (including those inside Form XObjects). For each:
   - `DCTDecode` (JPEG): decode with zune-jpeg, re-encode with mozjpeg at quality q, downscale if `pixels > cap(level)`.
   - `FlateDecode` with DeviceRGB or DeviceGray at 8 bits per component, no `/SMask` complication: decode, then JPEG at quality q if Photo-class, or re-deflate if Graphic-class.
   - Images with `/SMask`: recompress the base image as above; the soft mask stays Flate (re-deflated).
   - JBIG2, JPX (JPEG 2000), CCITT, Indexed, CMYK, ICCBased with N=4, 16-bit, and images used as masks: left untouched in v1.
   - Replace the stream only if the new one is smaller.
4. **Search.** One global setting `(q, cap)` for all images, walked down this list until the file fits: `(85, none)`, `(75, 4000 px)`, `(65, 3000)`, `(55, 2400)`, `(50, 2000)`, `(45, 1600)`, `(45, 1200)`. Each step reuses decoded images from a cache. The pixel cap is the long edge. If the last step does not fit, refuse with `BelowQualityFloor` and the smallest size reached.
5. Verify: reload from bytes with lopdf, page count unchanged, every page's content streams decompress, every replaced image decodes, file size under the limit. Text never changes, because content streams are copied byte for byte apart from recompression.

## 3.8 Office document planner

Applies to docx, docm, pptx, pptm, xlsx, xlsm, odt, odp, ods, and epub. These are ZIP files; the planner recompresses embedded media and the ZIP itself, and never edits XML text.

1. Open with the `zip` crate. Refuse password-protected OOXML (it is an OLE compound file, not a ZIP; detect the CFB signature and refuse with `Encrypted`).
2. Media entries: everything under `word/media/`, `ppt/media/`, `xl/media/`, `Pictures/` (ODF) and image files referenced from the EPUB manifest.
   - Images keep their format and extension, so `[Content_Types].xml` and relationship files never change. JPEG goes through the 3.4.4 search; PNG goes through oxipng and, in Fit mode, palette quantisation when that is at least 30 percent smaller and the image is Graphic-class. EMF/WMF/SVG are left as they are.
   - Embedded video (`.mp4`, `.mov`, `.m4v` in pptx) goes through the video planner with a budget share, keeping MP4/H.264. On the web and on desktop without FFmpeg, video entries are left unchanged and the prediction says so ("1 video left as is").
   - Embedded audio follows the audio planner, keeping the same container.
3. Budget split: non-media entries are measured after recompression at deflate level 9 and treated as fixed. The remaining budget is split across media entries with the allocator in 3.9.1.
4. Write a new ZIP with the same entry order. ODF and EPUB: `mimetype` first and stored uncompressed, as their specs require. XML entries: deflate level 9 (zopfli for entries under 1 MB). Already-compressed media: stored (method 0) when deflate saves under 1 percent.
5. Verify: re-open, CRC-check every entry by reading it fully, same entry names in the same order, every `.xml`/`.rels` entry parses with quick-xml, every relationship target that points inside the package exists, every replaced image decodes, size under the limit.

## 3.9 Multi-file jobs, archives and everything else

### 3.9.1 Budget allocation for a per-message limit

Used for Email (all attachments in one message), for documents with several media parts, and for archive packaging.

```
B = hard_bytes - packaging_overhead(names)            // exact for zip, see 3.9.2
pass 0 (parallel): for every item compute
    L_i = best lossless/optimised size (or original if nothing helps)
    F_i = size at the quality floor at full size (images: q_min encode; video: lowest rung at floor; audio: floor bitrate; pdf: last search step)
    fixed items (Text/Other/Archive that cannot shrink further): F_i = L_i
if sum(L) <= B: every item takes L_i. Done.
if sum(F) > B: refuse TotalTooBig, with SplitIntoMessages groups from first-fit-decreasing on F_i into bins of size B,
               and RemoveFiles listing the largest items whose removal makes the rest fit.
otherwise water-fill:
    spare = B - sum(F)
    b_i = F_i + spare * (L_i - F_i) / sum(L - F)            // proportional to how much each item can use
    clamp b_i to L_i and redistribute any excess, repeat until stable
round 1: encode every item to b_i (3.4 to 3.8 with budget b_i)
leftover = B - sum(actual)
round 2 (only if leftover >= 5% of B): give leftover to items whose quality label is below Good, proportionally to b_i; re-run those items
```

If `max_files_per_message` is set (Discord 10, Teams 10) and there are more outputs than that with SeparateFiles packaging, the plan switches to one ZIP and says why: "Discord takes 10 files per message, so Smidge will put these 23 files in one zip." For Discord's per-file limit the zip itself must fit the per-file limit.

When a per-message job does not fit and the SplitIntoMessages suggestion is accepted, the job is re-planned as N independent groups, each with its own budget B, and outputs go into `Name (Email 1 of 2)` folders or zips. Each group can be opened on its own; Smidge never produces split archive volumes.

### 3.9.2 Packaging

`Packaging::Auto` resolves as follows:

- One input file: no archive.
- Folder or several files, per-file limit (Discord, WhatsApp, Slack...): separate files when there are at most `max_files_per_message` outputs, otherwise one ZIP.
- Several files, per-message limit (Email): separate files when there are at most 10 outputs, otherwise one ZIP. The Compose screen shows the choice as a two-option switch: "Send as: 23 separate files | 1 zip file".
- Smaller mode with "Package into one file" on: ZIP.

Formats:

| Option | Writer | Method | Who can open it | Default |
|---|---|---|---|---|
| Zip (works everywhere) | `zip` crate | Deflate level 9 via zlib-rs; zopfli for entries under 1 MB; Stored for entries that shrink less than 1% | every OS without extra software | yes |
| 7z (smaller) | sevenz-rust2 | LZMA2, preset 9, solid, 64 MiB dictionary on desktop and 16 MiB on web | Windows 11 Explorer, 7-Zip, Keka, The Unarchiver | no |
| .tar.zst (fastest, technical) | tar + zstd | zstd level 19, long mode 27 | Linux/macOS command line, 7-Zip 23+ | no |
| .tar.xz | tar + liblzma | xz preset 9e | Linux/macOS | no |

Zip method 93 (zstd) and AES encryption are not offered: recipients' built-in tools can't open them, which breaks the "anyone can open it" promise.

ZIP overhead is computed exactly before encoding: per entry 30 + n bytes local header and 46 + n bytes central directory entry (n = UTF-8 name length), plus 20 bytes of Zip64 extra fields per entry when any size exceeds 4 GiB, plus 22 bytes end record (98 with Zip64). Names are UTF-8 with the language-encoding flag (bit 11) set, so non-English names survive on Windows.

### 3.9.3 Archive inputs

ZIP, 7z, tar, tar.gz, tar.bz2, tar.xz, tar.zst and gz are read with zip, sevenz-rust2, tar, flate2, bzip2, liblzma and zstd. RAR is not supported (the unRAR licence forbids building a RAR compressor from its code and is not an OSI licence; avoiding it is simpler). With `optimise_inside_archives` on (default), the archive is expanded into a temporary folder, its contents become the items of a per-message sub-job against the archive's budget, and the result is repacked in the same format, except that RAR-style unsupported methods and encrypted entries make the item `UnsupportedInput`.

### 3.9.4 Text and Other

- Text: in Fit mode, a single text file over the limit becomes a ZIP of that file if the ZIP fits ("big-log.txt → big-log (Discord).zip"). In Smaller mode with packaging off, text files are left as they are; they only shrink inside an archive.
- Other (executables, disk images, already compressed media containers we cannot open, encrypted files): try ZIP Deflate and 7z LZMA2 on the first 4 MiB; if neither saves 3 percent, the item is `CannotShrinkType`. A single Other file that is over the limit is refused: "Smidge can't make this kind of file smaller. It's 48.2 MB and Discord allows 20 MB." with PickPreset suggestions.

## 3.10 Output location and naming

Rules (MUST):

1. Never modify, move or overwrite any input or any existing file.
2. Desktop default: the output goes next to the original. Settings → "Where to save" can choose a fixed folder instead.
3. Name: `<stem> (<Label>).<ext>` where Label is the preset's `output_label` ("Discord", "Email", "WhatsApp", "Work email", "Custom 8 MB", "smaller"). Extensions follow the output format: `.jpg`, `.png`, `.webp`, `.avif`, `.gif`, `.mp4`, `.webm`, `.mp3`, `.m4a`, `.ogg`, `.flac`, `.wav`, `.pdf`, the original document extension, `.zip`, `.7z`, `.tar.zst`, `.tar.xz`.
4. Collisions: `<stem> (<Label> 2).<ext>`, then 3, and so on.
5. Folder input: a sibling folder `<folder> (<Label>)/` mirroring the relative paths; or `<folder> (<Label>).zip` when packaged.
6. A single input that is `KeptOriginal` produces no new file; the result says "Already fits. Nothing to change." and its buttons act on the original. Inside folder jobs, kept originals are copied into the output folder so the folder is complete.
7. Characters: names keep the user's characters, including emoji and non-Latin scripts. On Windows, a final name longer than 255 UTF-16 units or a full path beyond 32,767 is shortened by trimming the stem and adding `…`. Reserved Windows names (`CON`, `NUL`, ...) get a trailing `_`.

Atomic no-overwrite write on desktop (`cia-engine/src/output/fs_sink.rs`):

```
tmp = <dest_dir>/.<stem>.smidge-<job_id>.partial        // same directory, so same filesystem
write all bytes to tmp, fsync
loop n in 1..:
    dest = name(n)
    match hard_link(tmp, dest):                          // fails with AlreadyExists, never replaces
        Ok      -> remove(tmp); return dest
        Exists  -> continue
        Unsupported (FAT32, exFAT, some network shares) ->
            open dest with OpenOptions::create_new(true); on AlreadyExists continue
            copy tmp into it, fsync, remove(tmp); return dest
```

Partial files are removed on cancel, failure and at next launch (any `.smidge-*.partial` older than one hour in the output folders used by the last 50 jobs, recorded in `state.json`).

Web: outputs are written to OPFS under `/jobs/<job_id>/` with the same naming. "Download" saves one file directly, or a ZIP named `<folder> (<Label>).zip` for several (a ZIP is generated only for download convenience when the user has not chosen ZIP packaging; that ZIP is not measured against the limit and the UI does not call it the result). On Chromium, "Save into a folder" writes the files with `FileSystemDirectoryHandle.getFileHandle(name, { create: true })` after checking with `getFileHandle(name)` that the name is free, using the same collision numbering.

## 3.11 Verification and the honesty rule

An item may be reported as `Fitted` only if all of these hold for the bytes that were actually written:

1. The file exists at the reported location and its size, read back from the filesystem (desktop) or OPFS (web), is at most `hard_bytes` (which already has the preset's `safety_bytes` and any encoding overhead taken off, as computed below).
2. For per-message limits, the sum over all written outputs (or the archive) is under the limit too.
3. The type-specific checks pass:

| Type | Checks |
|---|---|
| Image | decodes with a different decoder from the encoder (`image` crate for mozjpeg/libwebp output, `zune-png` for oxipng output); dimensions as planned; alpha present if the source had alpha; no EXIF Orientation other than 1; GPS absent unless kept |
| Animated image | decodes; frame count as planned; total duration within 50 ms of source |
| Video | ffprobe (desktop) or mediabunny (web) reads it with no errors; codec and container allowed by the preset; duration within max(0.1 s, one frame) of source or trim; aspect ratio within 1 percent; portrait stays portrait; audio stream present iff the source had audio and sound was kept; first and last second decode |
| Audio | full decode; duration within 0.1 s; channels as planned |
| PDF | 3.7 step 5 |
| Office | 3.8 step 5 |
| Archive | re-open; every entry CRC-checked; entry count and names as planned |

4. If any check fails, the attempt is a failure and the retry logic decides what happens next. The UI never shows a green "Fits" state for an item that has not passed all checks.

The outcome is exactly one of the `ItemOutcome` variants, and the result screen copy is generated from it in `cia-core/src/copy.rs` (one function per variant), so the UI cannot invent its own success message.

How `hard_bytes` is computed from a preset limit (`cia-core/src/limits.rs`):

```
counts = "raw":          hard_bytes = limit_bytes - safety_bytes - 1     // "-1": the service's limit is treated as exclusive
counts = "mime_base64":  hard_bytes = floor((limit_bytes - 100_000) * 3/4 * 76/78) - safety_bytes
                          // base64 is 4/3 larger, plus CRLF every 76 characters; 100 kB for headers and body
```

For Gmail's 25 MB per message this gives 18,196,153 bytes of attachments, shown as "18.2 MB". That is lower than the 25 MB people expect, so the Email tile shows "Up to 18 MB of attachments" and the info tooltip explains: "Email adds about a third to every attachment while sending, so a 25 MB email holds about 18 MB of files."

## 3.12 Events and logging

Events (Rust enum, serde tag `type`, mirrored in TS by ts-rs):

```rust
pub enum EngineEvent {
    JobState   { job_id: JobId, state: JobState },
    ItemState  { job_id: JobId, item_id: ItemId, state: ItemState },
    Progress   { job_id: JobId, item_id: ItemId, fraction: f32, eta_ms: Option<u64>, label: String },
    Attempt    { job_id: JobId, attempt: Attempt },
    ItemDone   { job_id: JobId, item_id: ItemId, outcome: ItemOutcome },
    JobDone    { job_id: JobId, summary: JobSummary },
    Warning    { job_id: JobId, item_id: Option<ItemId>, message: String },
}
```

Desktop emits them on the Tauri event `engine://event`; web workers post them to the main thread. Progress events are throttled to 10 per second per item.

Logs:

- Job log: one JSON file per job with the Job, Plan, every Attempt, every ItemOutcome, capabilities, FFmpeg version and encoder chain used, and timings. Desktop: `<app_data>/logs/jobs/<yyyymmdd-hhmmss>-<job_id>.json`, the newest 200 kept. Web: kept in memory for the session.
- App log: `tauri-plugin-log` to `<app_data>/logs/app.log`, 5 MB, 3 rotations.
- "Copy diagnostic report" (Settings → About) saves a ZIP with the last 10 job logs, app.log and capabilities, and opens its folder so the user can attach it to an email to support. File names inside job logs are included; the dialog says so before saving.
- No telemetry, no crash reporting service, no analytics in either app. The only network calls are the license API, the FFmpeg download (with consent) and the update check.

## 3.13 Preset schema and data

`packages/presets/presets.json` is the single source of truth, embedded into the Rust engine with `include_str!` and validated at build time by a test that also checks every `source.checked_on` is within 120 days of the build date (warning) or 365 days (error). `cargo xtask presets-doc` writes `docs/presets-sources.md` from it.

JSON Schema (`packages/presets/presets.schema.json`, draft 2020-12), shown here as an annotated example of one entry:

```json
{
  "schema_version": 1,
  "presets": [
    {
      "id": "discord-free",
      "group": "discord",
      "tile_label": "Discord",
      "tier_label": "Free",
      "output_label": "Discord",
      "icon": "discord",
      "order": 10,
      "limit": {
        "bytes": 20971520,
        "stated_as": "20 MiB",
        "scope": "per_file",
        "counts": "raw",
        "safety_bytes": 65536
      },
      "limit_by_kind": {},
      "max_files_per_message": 10,
      "formats": {
        "image": ["jpeg", "png", "webp", "gif"],
        "animated": ["gif", "webp", "mp4"],
        "video": [
          { "container": "mp4", "video": "h264", "audio": "aac" },
          { "container": "webm", "video": "vp9", "audio": "opus" }
        ],
        "audio": ["mp3", "ogg_opus", "m4a_aac", "flac"],
        "document": "keep"
      },
      "video_caps": { "max_short_edge": 1080, "max_fps": 60 },
      "source": {
        "urls": [
          "https://support.discord.com/hc/en-us/articles/25444343291031-File-Attachments-FAQ",
          "https://docs.discord.com/developers/reference"
        ],
        "checked_on": "2026-10-02",
        "confidence": "official",
        "note": "Raised from 10 MB in August 2026. Developer docs state 20 MiB per file; a file of exactly the limit is rejected (discord-api-docs #7603), hence safety_bytes."
      }
    }
  ]
}
```

Field rules: `scope` is `per_file` or `per_message`; `counts` is `raw` or `mime_base64`; `confidence` is `official`, `third_party` or `community`; `formats.*` lists are in preference order; `limit_by_kind` keys are `image`, `video`, `audio`, `document`, `other` and override `limit` for that kind.

All presets (bytes are what the engine uses; checked 2026-10-02):

| id | Tile / tier | limit.bytes | scope | counts | safety | Allowed video | Notes and source |
|---|---|---|---|---|---|---|---|
| discord-free | Discord / Free | 20,971,520 | per_file | raw | 65,536 | mp4 h264, webm vp9 | 20 MiB per file. support.discord.com File Attachments FAQ (edited 2026-09-30); docs.discord.com/developers/reference |
| discord-nitro-basic | Discord / Nitro Basic | 50,000,000 | per_file | raw | 65,536 | same | "50MB". Unit not stated, so decimal (smaller, safe). Same FAQ; support.discord.com/hc/en-us/articles/115000435108 |
| discord-nitro | Discord / Nitro | 1,000,000,000 | per_file | raw | 1,000,000 | same | "up to 1GB", raised from 500 MB. Same FAQ |
| discord-boost-2 | Discord / Server boost level 2 | 50,000,000 | per_file | raw | 65,536 | same | Applies in that server only. support.discord.com/hc/en-us/articles/360028038352 |
| discord-boost-3 | Discord / Server boost level 3 | 100,000,000 | per_file | raw | 65,536 | same | as above |
| email | Email / Gmail, Outlook, Yahoo | 25,000,000 | per_message | mime_base64 | 0 | mp4 h264 | Gmail "the limit is 25 MB" per message (support.google.com/mail/answer/6584); Outlook.com "25 MB" (support.microsoft.com, "Sending limits in Outlook.com"); Yahoo "must not exceed 25MB in total" (help.yahoo.com/kb/SLN5673.html). hard_bytes 18.2 MB |
| email-icloud | Email / iCloud Mail | 20,000,000 | per_message | mime_base64 | 0 | mp4 h264 | "20MB" per message (support.apple.com/102198). hard_bytes 14.5 MB |
| work-email | Work email / Microsoft 365 | 35,000,000 | per_message | mime_base64 | 0 | mp4 h264 | Exchange Online default 35 MB send (learn.microsoft.com, Exchange Online limits); admins can lower it, so the tile notes "Your company may use a lower limit. Use Custom if you know it." hard_bytes 25.5 MB |
| whatsapp | WhatsApp | 64,000,000 | per_file | raw | 1,000,000 | mp4 h264 | Video default 100 MB/720p on fast connections, 64 MB/480p on slow ones; we use 64 MB and max short edge 720 (faq.whatsapp.com/453914586839706). `limit_by_kind.document` = 2,000,000,000 ("maximum allowed document size is 2 GB", faq.whatsapp.com/545917406122452) |
| imessage | iMessage | 100,000,000 | per_file | raw | 10,000,000 | mp4 h264 | Apple publishes no number; about 100 MB is a community figure (discussions.apple.com/thread/252822623). confidence "community"; effective 90 MB |
| slack | Slack | 1,000,000,000 | per_file | raw | 1,000,000 | mp4 h264 | "files up to 1GB" (slack.com/help/articles/201330736) |
| teams | Microsoft Teams | 100,000,000 | per_file | raw | 1,000,000 | mp4 h264 | Chat "File size limitation: 100 MB", up to 10 attachments (learn.microsoft.com/microsoftteams/limits-specifications-teams, updated 2026-10-01) |
| telegram | Telegram / Free | 2,000,000,000 | per_file | raw | 1,000,000 | mp4 h264, webm vp9 | "each up to 2 GB" (telegram.org/blog/700-million-and-premium) |
| telegram-premium | Telegram / Premium | 4,000,000,000 | per_file | raw | 1,000,000 | same | "send 4 GB files" (same) |
| custom | Custom size | user value | user toggle, default per_file | raw | 0 | mp4 h264 | Number field plus unit KB/MB/GB (decimal), and "Each file / All files together" |
| smaller | Just make it smaller | none | n/a | n/a | n/a | keeps source codec family where possible | Smaller mode |

Allowed image formats: Email, iCloud, Work email and iMessage use `jpeg, png, gif`; WhatsApp `jpeg, png`; Discord, Slack, Teams and Telegram `jpeg, png, webp, gif`. Allowed audio: Email, Work email, iMessage and WhatsApp `mp3, m4a_aac, flac`; Discord, Slack, Telegram and Teams `mp3, ogg_opus, m4a_aac, flac`.

---
# 4. UI and UX

All screens live in `packages/ui` and render the same in the desktop app and the web app. Host differences are capability flags on `EngineHost`, never separate screens.

## 4.1 Brand

- **Name:** Smidge. Wordmark in Bricolage Grotesque Bold, lowercase "smidge", with a small dot-in-a-square mark (a big square containing a small one: the file made small).
- **Accent:** Smidge Violet `#6D4AFF` (hover `#7F60FF`, pressed `#5A37EE`), white text on it (contrast 5.2:1). Violet is the action colour only: buttons, focus rings, selected tiles, links.
- **Verdict colours are separate from the accent.** Green means "fits" and nothing else: `#2FBF71` dark theme, `#1E9E5A` light theme. Red means "doesn't fit" or error: `#F0566A` dark, `#D2304A` light. Amber `#F2A93B` for warnings. Keeping the brand colour out of the verdict means a violet button can never be mistaken for a success state.
- **Fonts:** Inter for UI and body text (as ConvertSave), Bricolage Grotesque for headings and the wordmark (both SIL OFL 1.1, bundled via `@fontsource/inter` and `@fontsource-variable/bricolage-grotesque`, never loaded from a CDN).
- **Type scale (kept from ConvertSave):** `type-1` 1.875rem / 2.25rem, `type-2` 1.2rem / 1.75rem, `type-3` 0.875rem / 1.375rem. Headings weight 700 (Bricolage), Inter at 0.875rem weight 500, form controls Inter 0.875rem / 1.375rem / 500.
- **Tokens:** `packages/tokens/src/tokens.ts` uses ConvertSave's `ThemeTokenSet` key list unchanged (copy the type from `ConvertSave/src/styles/tokens.ts`), with these values:

| Token | Dark | Light |
|---|---|---|
| surface-base | `#14121B` | `#F6F4F0` |
| surface-card | `#1D1A27` | `#FFFFFF` |
| surface-overlay | `#272336` | `#FFFFFF` |
| surface-sunken / surface-recessed | `#100E16` | `#EEEBE6` |
| surface-hover | `#272336` | `#F0EDE8` |
| on-surface | `#ECEAF4` | `#1C1924` |
| on-surface-muted | `#B1ACC4` | `#5C5768` |
| on-surface-subtle | `#77718C` | `#8A8496` |
| on-surface-disabled | `#4A4566` | `#C4BEB6` |
| primary / accent / focus-ring / form-accent | `#6D4AFF` | `#6D4AFF` |
| on-primary / on-accent | `#FFFFFF` | `#FFFFFF` |
| primary-container | `#2A2150` | `#EEEAFF` |
| on-primary-container | `#DCD4FF` | `#2B1C7A` |
| success (fits) | `#2FBF71` | `#1E9E5A` |
| success-container | `#123826` | `#DDF5E8` |
| danger | `#F0566A` | `#D2304A` |
| danger-container | `#3D1820` | `#FBE0E5` |
| warning | `#F2A93B` | `#B86E00` |
| warning-container | `#3A2A10` | `#FFF1D6` |
| border-subtle | `#262233` | `#ECE8E2` |
| border-default | `#34304A` | `#D6D0C8` |
| border-strong | `#4A4566` | `#BDB5AB` |
| scrim | `rgba(8, 6, 14, 0.62)` | `rgba(28, 25, 36, 0.32)` |

  Remaining keys (hover/pressed/disabled variants, shadows) are derived exactly as ConvertSave derives them. `index.css` variables must match `tokens.ts`; a unit test compares them.
- **Tone of voice:** plain words, short sentences, real numbers. Say what happened and what to do next. No exclamation marks, no "Oops", no blame ("This file is damaged", not "You chose a bad file"), no jargon on the main screen (no "bitrate", "codec", "CRF"; Advanced may name codecs). Numbers are always "19.6 MB", one decimal, decimal megabytes, with "about" when the number is a prediction. Destination limits are the exception: they are shown the way the service states them ("Discord Free, 20 MB"), even when the engine uses the exact binary value.

## 4.2 Window and layout

Desktop window: default 1040 × 720, minimum 760 × 560, remembered. One main view with a header:

```
+----------------------------------------------------------------------------+
| [mark] smidge                           2 free files left today   (?)  [⚙] |
+----------------------------------------------------------------------------+
|  1  Add your files                                                         |
|  +----------------------------------------------------------------------+  |
|  |            Drop files or a folder here                               |  |
|  |            or  [Choose files]  [Choose folder]                       |  |
|  +----------------------------------------------------------------------+  |
|                                                                            |
|  2  Where are you sending it?                                              |
|  [Discord v] [Email v] [WhatsApp] [iMessage] [Slack] [Teams] [Telegram v]  |
|  [Work email] [Custom size]                    [Just make it smaller]      |
|                                                                            |
|  3  Ready                                                                  |
|  1 min 10 s video -> 720p, 30 fps, about 19.2 MB. Quality: Good.           |
|  [ Compress ]                                         Advanced  >          |
+----------------------------------------------------------------------------+
```

The three steps are always visible on one screen (no wizard pages), numbered, and unlock visually in order: the destination tiles are clickable at any time, Compress is enabled once there are files, a destination, and a plan that is not `CannotFit`. The header pill shows the free allowance (hidden for Pro). `(?)` opens Help (Show the tour again, Help articles on the site, Contact support). The gear opens Settings.

Tiles with a `v` open a small tier menu: Discord (Free 20 MB, Nitro Basic 50 MB, Nitro 1 GB, Server boost level 2 50 MB, Server boost level 3 100 MB), Email (Gmail, Outlook, Yahoo: up to 18 MB of attachments; iCloud Mail: up to 14 MB), Telegram (Free 2 GB, Premium 4 GB). The tier choice is remembered per group. Custom size opens an inline field: number, unit (KB/MB/GB), and "Each file / All files together".

The web app is the same layout, responsive down to 360 px wide (tiles wrap to two columns; the drawer becomes a bottom sheet).

## 4.3 The main flow, state by state

**Empty.** Drop zone with the line "Your files stay on this computer." (web: "Your files stay in this browser. Nothing is uploaded."). Destination tiles visible. Step 3 says "Add files to see what you'll get."

**Files added.** The drop zone shrinks to a file list. Each row: type icon, name, size, a short detail ("4032 × 3024 photo", "4 min 12 s video", "12-page PDF"), and a remove button. Folders show as one row with a count ("Grandkids · 23 photos · 91.3 MB") that expands. Video rows also get a "Trim" button. Rows that need FFmpeg on desktop show "Needs video support" with a "Set up" button (4.8). Rows the host can't handle show the reason inline (4.10).

**Destination picked.** Within 300 ms the prediction appears in step 3 (the engine's `preview`). Single file: "1 min 10 s video → 720p, 30 fps, about 19.2 MB. Quality: Good." Several files: "23 photos → about 17.1 MB in 1 zip file, fits in one email. Quality: Good." plus the "Send as: separate files | 1 zip file" switch when 3.9.2 offers it. Predictions for video and PDF that require analysis show "Working out the best settings…" with a spinner until ready; images run their real pass-0 encodes, so the number is usually exact.

**Can't fit (before encoding).** Step 3 turns into a red-bordered card: the Refusal message and its suggestions as buttons. Example: "Too long to fit in 20 MB at watchable quality." [Trim to under 4 min 50 s] [Use Nitro Basic (about 48 MB)] [Use WhatsApp]. Compress stays disabled.

**Compressing.** The Compress button becomes a progress bar with "About 1 min 10 s left" and a Cancel button. Each row shows its own state: Waiting, Working (with a thin bar), Checking, Done (size), Couldn't fit, Failed. Cancel asks nothing; it stops immediately and removes partial files, then shows "Stopped. Nothing was saved." (or "Stopped. 12 files were saved before you stopped." for folder jobs with per-file limits).

**Done, everything fits.** A results card replaces step 3:

```
  ✓ Fits in Discord                                   (green check icon, not an emoji)
  380.0 MB → 19.2 MB
  Jayden clip (Discord).mp4 · 720p · 30 fps · Quality: Good

  [ Copy file ]   [ Show in folder ]   [ Play ]      ⠿ Drag into Discord
                                                         Compress something else
```

Desktop: "Copy file" puts the file on the clipboard as a file (Windows CF_HDROP, macOS file URL on NSPasteboard, Linux `text/uri-list` plus `x-special/gnome-copied-files`) using `clipboard-rs`; "Show in folder" reveals and selects it; "Play"/"Open" opens with the default app; the drag handle starts an OS drag of the file (`tauri-plugin-drag`). Multi-file results show "Copy files", "Show folder", and the list. Web: "Download" and, on Chromium, "Save into a folder". Web has no copy button because browsers cannot put arbitrary files on the clipboard.

**Done, some fit.** Amber header: "18 of 23 files fit. 5 couldn't." Fitted files are saved; each refused one is listed with its own message and suggestions. Applies only to per-file limits (per-message jobs are all-or-nothing).

**Done, nothing fit.** Red card, same layout as "Can't fit", with the smallest size reached: "The smallest Smidge could make this was 23.8 MB. Discord allows 20 MB." Nothing is saved.

**Failed.** Red card with the Failure message and "Copy details for support" (copies the job log path and summary).

Keyboard: Ctrl/Cmd+O choose files, Ctrl/Cmd+Enter compress, Esc cancel or close drawer, Ctrl/Cmd+, settings. Progress uses an `aria-live="polite"` region. Respect `prefers-reduced-motion`.

## 4.4 Trim

Video and audio rows have "Trim". It opens a panel under the row: a timeline with a start and an end handle, a preview frame for the handle being dragged (desktop: FFmpeg extracts a JPEG frame at that time, debounced 150 ms; web: `VideoDecoder` decodes the nearest keyframe and seeks), the selected length, and the live prediction. Refusal suggestions "Trim to under X" open this panel with the end handle preset to X. Audio rows show a waveform instead of frames (computed from a 1 kHz mono decode).

## 4.5 Advanced drawer

Opens from "Advanced" in step 3 as a right-side drawer (bottom sheet on narrow web). Every setting has a one-line explanation under it. Defaults in brackets. Sections show only when relevant files are in the list.

- **Photos:** Limit photo size [off] (long edge: 4096 / 3000 / 2048 / 1600 / 1280 px); Keep photo details like date and camera [off]; Keep location [off, enabled only with the previous]; Allow format changes, e.g. PNG photo to JPEG [on for destinations, off for Just make it smaller]; Modern formats WebP/AVIF in Just make it smaller [off]; Flatten transparency onto white [off].
- **Video:** Format [Most compatible (H.264 MP4)] / Smaller files (VP9 WebM, plays in Discord and browsers); Faster, uses your graphics card [on]; Sound [Mix all tracks] / Track 1 / Track 2 ... / Remove sound; Frame rate [Automatic] / Keep original / 30 fps.
- **Music and audio:** Format [Automatic] / MP3 / AAC / Opus / FLAC.
- **Documents and PDFs:** Keep document details (author, title metadata) [on].
- **Folders and archives:** Packaging [Automatic] / Separate files / Zip (works everywhere) / 7z (smaller) / .tar.zst; Optimise files inside zip files [on].
- **Output:** Save next to the original [default] / Always save to… (folder picker). Desktop only. File names always follow 3.10 (`photo (Discord).jpg`, then `photo (Discord 2).jpg`); nothing is ever overwritten.

Drawer settings apply to the current job and are remembered as the new defaults, except trims.

## 4.6 Spotlight tutorial

Copy ConvertSave's `src/components/SpotlightTutorial.tsx` (and `src/lib/zoomRatio.ts`, which it imports) into `packages/ui/src/components/SpotlightTutorial.tsx` unchanged except for the class prefix (`noc-` becomes `smg-`) and the dialog label. Targets are `[data-tour="..."]` attributes, never class names. Runs automatically on first open after the welcome card, and again from Help → "Show the tour again". Completion or skip is stored as `tour_version_seen` in settings; bumping `TOUR_VERSION` re-runs it once after a major UI change.

| # | Target | Title | Body |
|---|---|---|---|
| 1 | none (centred) | Welcome to Smidge | Smidge makes files small enough to send. This tour takes about 20 seconds. |
| 2 | `dropzone` | 1. Add your files | Drag files or a whole folder here, or click to choose them. Your files never leave this computer. (web: "never leave this browser") |
| 3 | `destinations` | 2. Pick where it's going | Each button knows that app's size limit: Discord, email, WhatsApp and more. If you know your own limit, use Custom size. |
| 4 | `smaller` | No limit in mind? | Just make it smaller shrinks everything as much as it can without visible loss. |
| 5 | `prediction` | 3. Check, then compress | Before it starts, Smidge tells you the size and quality you'll get. If something can't fit, it says so here and suggests a fix. |
| 6 | `advanced` | Extra options | Photo size, video sound, formats and where files are saved live here. You never have to open it. |
| 7 | none (centred) | You're ready | When it's done, copy the file or drag it straight into Discord, WhatsApp or your email. |

## 4.7 First run (desktop)

1. Welcome card (centred, over the main screen): wordmark, "Make any file small enough to send.", buttons [Show me around] (starts the tour) and [Skip].
2. No license screen, no account, no FFmpeg prompt at first run. Images, PDFs, documents, archives and lossless audio work immediately.
3. The FFmpeg setup appears only when the user adds a file that needs it, or opens Settings → Video support.

Web first run is the same, plus a one-time dismissible banner when the browser lacks something important: "This browser can't make videos smaller. Chrome, Edge or Safari can, or use the desktop app."

## 4.8 FFmpeg setup flow (desktop)

Trigger: a row says "Needs video support" and the user clicks "Set up", or Settings → Video support → "Add video support".

Modal, exact copy:

> **Add video and music support**
>
> Smidge uses FFmpeg to work with videos and some music files. FFmpeg is a free, open-source program used by many apps. Smidge doesn't include it, so it needs to download it once.
>
> - Download: about 34 MB, from Smidge's public build page on GitHub
> - FFmpeg runs as a separate program on this computer
> - Your files still never leave this computer
>
> FFmpeg is licensed under the LGPL. [What does that mean?]
>
> [Download FFmpeg]   [Not now]
>
> Already have FFmpeg? [Use my own copy…]

The size comes from the manifest, not hard-coded. "What does that mean?" expands: "FFmpeg is made by the FFmpeg project, not by Smidge. The build Smidge downloads leaves out every part that isn't under the LGPL licence. Its exact build settings and source code are published at github.com/Hunter-Boone/Smidge-Libraries."

States: "Downloading FFmpeg… 12 of 34 MB" (progress bar, Cancel) → "Checking the download…" (SHA-256 against the signed manifest, section 6.4) → "Testing video encoders…" (3.5.6) → "Video support is ready." The modal closes and the rows re-plan.

Errors:

- Network: "The download didn't finish. Check your internet connection and try again." [Try again]
- Hash or signature mismatch: "The download was damaged, so Smidge deleted it. Please try again." Two mismatches in a row: "Smidge couldn't get a good copy of FFmpeg. Contact support and we'll help." (with diagnostic report button)
- Disk full: "There isn't enough free space. FFmpeg needs about 90 MB."
- Antivirus quarantine (file vanishes or cannot execute after unpack): "Your security software blocked FFmpeg. You can allow it, or use your own copy." [Use my own copy…]

"Use my own copy…" opens a file picker for the `ffmpeg` executable; Smidge looks for `ffprobe` next to it, runs `-version` and the encoder tests, and shows the version. Using the user's own FFmpeg is how a user can get `libx264`; Smidge never downloads a GPL build.

Install location: `<app_data>/tools/ffmpeg/<version>/` with `ffmpeg[.exe]`, `ffprobe[.exe]`, `LICENSE.md`, `BUILD_CONFIG.txt`, `manifest.json`. Remove deletes that folder. Smidge downloads with its own HTTP client, so macOS sets no quarantine attribute and Windows adds no mark-of-the-web; the binaries run as normal child processes (6.4).

## 4.9 Settings

Desktop sections (left list, content right):

- **General:** Where to save (Next to the original / Always save to…); Theme (Match system / Light / Dark); Text size (100 / 115 / 130 percent, ConvertSave's zoom approach); Show the tour again.
- **Video support:** status line ("Ready · FFmpeg 7.1.2 · H.264 via NVIDIA"), list of working encoders, [Re-test encoders], [Use a different FFmpeg…], [Remove video support], Faster mode default.
- **License:** plan and status ("Smidge Pro · Lifetime" / "Free · 3 files a day"), the key masked as `XXXXX-…-7KQ2P` with Show and Copy, "This computer is 1 of 3", [Deactivate this computer], [Manage my account] (opens `www.<domain>/account`), [Buy Smidge Pro] when free, [Enter a product key].
- **Updates:** Check automatically [on], channel (Stable / Beta), [Check now], current version, release notes link.
- **About:** version and build, [Licenses], privacy summary ("Smidge never uploads your files. It only connects to the internet to check your license, download FFmpeg when you ask, and check for updates."), [Copy diagnostic report], [Open logs folder], support email.

Web: General (minus "Where to save"), License, About, and a "This browser" section listing what it can and can't do (from `Capabilities.web`).

Settings file: `<app_data>/settings.json`, versioned (`"v": 1`), read and written only through `cia-desktop/src/settings.rs`; the web app stores the same JSON in IndexedDB (`idb-keyval`).

## 4.10 Error and edge states (exact copy)

| Situation | Message |
|---|---|
| Damaged file | "This file is damaged or incomplete, so Smidge can't read it." |
| Unsupported type for this host | "Smidge can't open this kind of file yet." |
| Needs FFmpeg (desktop) | "Videos need a one-time setup." [Set up] |
| Web, browser can't decode the video | "This browser can't read this video. Try Chrome, Edge or Safari, or the desktop app." |
| Web, HEIC on non-Safari | "This browser can't open iPhone HEIC photos. Use Safari, or the desktop app." |
| Encrypted PDF or document | "This file is password-protected, so Smidge can't change it." |
| Disk full while saving | "There isn't enough free space to save the result. Free up about 40 MB and try again." |
| Output folder not writable | "Smidge can't save in that folder. Choose another folder in Advanced." |
| Original deleted mid-job | "The original file was moved or deleted while Smidge was working." |
| Free allowance used up | "You've used your 3 free files. Your next free file is ready at 10:42, or get Smidge Pro for unlimited files." [Get Smidge Pro] [Enter a key] |
| Over the limit after all retries | "Smidge couldn't get this under 20 MB. The closest was 21.3 MB, so nothing was saved." |
| Kept original | "Already fits. Nothing to change." / "Already as small as it gets." |

## 4.11 Licenses page

Route `/licenses` inside the app (Settings → About → Licenses). Data from `packages/licenses-data/licenses.json`, generated by `cargo xtask licenses` from `cargo about generate` (Rust) and `license-checker-rseidelsohn --production --json` (npm), merged with a hand-written `extra.json` for things neither tool sees (FFmpeg, fonts, patent grants). Paddle.js runs only on the site, so it is not listed in the apps.

Groups, in this order:

1. **Downloaded with your permission:** FFmpeg (LGPL-3.0 full text, link to build config and source in Smidge-Libraries), its bundled codec libraries (libvpx BSD-3, libaom BSD-2, LAME LGPL-2.0, libopus BSD-3, libvorbis BSD-3, libwebp BSD-3, zimg WTFPL, SVT-AV1 BSD-3-Clause-Clear, nv-codec-headers MIT, AMF headers MIT, libvpl MIT, libva MIT).
2. **Built into Smidge:** every Rust crate and npm package that ends up in the shipped binary or bundle, with version, licence, copyright line, and full text on expand. MPL-2.0 entries (Symphonia, mediabunny) add: "Used unmodified. Source: <url>."
3. **Patent licences:** AOMedia Patent License 1.0 (rav1e, libaom, SVT-AV1), the WebM/VP8/VP9 patent grant (libvpx, libwebp), the Opus patent grants.
4. **Fonts:** Inter and Bricolage Grotesque, SIL OFL 1.1.

A search box filters by name. CI regenerates the data and fails if `licenses.json` is stale, and the release workflow copies it to `NOTICE.md` at the repo root and into the Downloads repo release notes.

## 4.12 Upgrade, activation and account entry points

- **Upgrade modal** (from the free pill, the allowance message, or Settings): "Smidge Pro" with the two plans (section 5.2) and what Pro adds ("Unlimited files, on up to 3 computers and in your browser"). [Buy Pro] opens the purchase flow (5.6). [I already have a key] opens Activate.
- **Activate modal:** one field that accepts a key typed or pasted in any case, with or without dashes; it formats as XXXXX-XXXXX-XXXXX-XXXXX while typing and checks the last character locally, so typos show "That key has a typo. Check the email we sent you." before any network call. [Activate]. Below: "Lost your key? [Email it to me]" (5.6.7).
- **Waiting for purchase** (after Buy): "Finish your purchase in the browser. Smidge will unlock by itself." with a spinner and "I'll enter my key instead".
- **Unlocked:** "Smidge Pro is on. Thanks for supporting Smidge." Auto-dismisses after 4 s.

---
# 5. Auth, licensing and payments

## 5.1 Decisions in one place

1. **No passwords, anywhere.** Customers identify themselves by product key (in the apps) or by a 6-digit email code (only on the account dashboard).
2. **Free tier, no trial.** Free: 3 files per rolling 24 hours per computer or browser, every feature and preset, no account, no watermark. Pro removes the limit. ConvertSave's card-up-front trials produced trial-abuse code, a renewal bug that cancelled paying customers, and "forgot to cancel" refunds; a free tier has none of those.
3. **Two plans:** Smidge Pro Lifetime (one-time price) and Smidge Pro Yearly (subscription). No monthly plan: people need Smidge occasionally, and a monthly plan invites cancel-after-one-use churn. Prices are Hunter's (section 9).
4. **Paddle Billing is merchant of record.** Paddle handles tax, receipts, refunds, card updates and cancellation in its customer portal. We never store card data and never build billing UI.
5. **A purchase becomes an entitlement in our database** through the Paddle webhook, keyed on Paddle's transaction id or subscription id. One entitlement has one product key.
6. **Licenses are signed, not encrypted.** The server signs a small license token with an Ed25519 private key; the apps verify it with the public key compiled in. ConvertSave compiled its AES key into the binary, so anyone who extracted it could mint licenses; a public key cannot mint anything.
7. **Device limit: 3 computers per license, plus up to 3 browsers.** Browsers do not use computer slots (browsers clear storage, which would burn slots); a fourth browser silently replaces the least recently used one.
8. **Offline works.** A desktop token is valid offline for 45 days from its last refresh. Past that, or if the clock is wound back, Smidge drops to the free tier until it can reach the server once. It never locks the user out of the free tier.
9. **The web app is gated client-side, on purpose.** All compression runs in the browser, so a determined user can bypass the check with developer tools. That is accepted: the gate exists for honest customers, the same free allowance applies, and the effort of bypassing it is worth more than the price.
10. **Refunds and chargebacks revoke automatically** via Paddle adjustment webhooks; access ends at the next refresh (at most 24 hours online, 45 days offline).

## 5.2 Plans and the free allowance

| | Free | Pro Lifetime | Pro Yearly |
|---|---|---|---|
| Price | 0 | one payment | per year |
| Files | 3 per rolling 24 h, per computer or browser | unlimited | unlimited while subscribed |
| Presets, formats, folders, packaging, Advanced | all | all | all |
| Computers | the one you're on | 3 | 3 |
| Web app | 3 per rolling 24 h per browser | unlimited, 3 browsers | unlimited, 3 browsers |
| Account needed | no | no (key only); dashboard optional | same |

Free allowance rules (`cia-core/src/allowance.rs`, used by both hosts):

- Counts only `Fitted` outcomes. Refusals, failures, cancellations and `KeptOriginal` never count, so a free user is never charged an allowance for a result that didn't work.
- Each use records a timestamp; a slot frees exactly 24 hours after it was used. The message gives the time: "You've used your 3 free files. Your next free file is ready at 10:42."
- A job with more files than remaining slots is blocked before it starts: "This has 23 files. The free version does 3 a day." [Get Smidge Pro] [Do the first 3 now]. "Do the first 3 now" removes the rest from the job.
- Desktop stores it in `<app_data>/usage.json`; web in IndexedDB. Both are tamperable. That is accepted (5.1 item 9).

## 5.3 Components

```
  Desktop app (Tauri)         Web app (app.<domain>, static, Cloudflare Pages)
        |  HTTPS JSON                  |  HTTPS JSON (CORS)
        v                              v
  +--------------------------------------------------------------+
  | Site: Next.js 15 on Vercel, www.<domain>                      |
  |  /buy /success /account  (pages)                              |
  |  /api/v1/*               (route handlers, Node runtime)       |
  +--------------------------------------------------------------+
        |               |                    |                |
        v               v                    v                v
   Supabase        Paddle Billing        Resend          GitHub (public
   Postgres        (API + webhooks)      (email)         Downloads repo:
   (service role                                          updater channels)
    from server only)
```

Supabase is a new project for this product (`smidge-prod`, plus `smidge-dev` for previews and sandbox), not ConvertSave's. Only the site's server code talks to it, with the service role key; RLS is enabled on every table with no policies, so the anon key can read nothing. The apps never hold a database credential, an API secret or a shared secret. ConvertSave's `x-convertsave-app` header is not repeated: a secret compiled into a public binary is not a secret, and rate limits do the real work.

## 5.4 Product key

- Format: `XXXXX-XXXXX-XXXXX-XXXXX`, the same look as ConvertSave's so support staff recognise it.
- Charset (31): `ABCDEFGHJKMNPQRSTUVWXYZ23456789` (no 0, O, 1, I, L).
- 19 random characters plus 1 check character. Random characters use rejection sampling (draw a byte, reject values >= 248, take value mod 31) so every character is uniform; ConvertSave's `byte % 31` has a slight bias.
- Check character: Luhn mod N with N = 31 over the 19 characters. This catches every single-character typo and most swapped neighbours, so the app can say "That key has a typo" without a network call.
- Entropy: 19 × log2(31) ≈ 94 bits.
- Normalisation before any check: uppercase, remove spaces and dashes. Input containing `0`, `O`, `1`, `I` or `L` fails as a typo.
- Implemented once in Rust (`cia-license/src/key.rs`) and once in TypeScript (`packages/api-types/src/product-key.ts`) with a shared test-vector file `packages/api-types/test-vectors/product-keys.json` that both test suites read.
- Storage: `product_key_hash = sha256(normalised)` for lookup, `product_key_enc` (AES-256-GCM with `KEY_ENCRYPTION_KEY`) so we can email it again, `key4` = last 5 characters for display. The full key is never logged.

## 5.5 License token

```
SMG1.<payload_b64url>.<signature_b64url>

signature = Ed25519(private_key[kid], ASCII("SMG1." + payload_b64url))
payload (JSON, UTF-8, keys in this order):
{
  "v": 1,
  "kid": "2026-10",                 // which public key verifies it
  "ent": "ent_01K6QW...",           // entitlement id
  "plan": "lifetime" | "yearly",
  "kind": "desktop" | "web",
  "dev": "d_k3j...",                // desktop device hash, or "w_..." web install id
  "iat": 1759400000,                // issued at, unix seconds
  "exp": 1763288000,                // offline validity end: iat + 45 days (desktop), iat + 14 days (web)
  "acc": null | 1790936000,         // paid-through time for yearly; null for lifetime
  "key4": "7KQ2P"
}
```

Public keys live in `crates/cia-license/src/keys.rs` as a list of `(kid, [u8; 32])`. The server signs with `LICENSE_SIGNING_KID`. Rotation: add the new public key to the app, ship it, wait until most installs have updated, then switch the server's kid. The old key stays in the list for one more major version.

Device id (`cia-license/src/device.rs`, native feature):

```
machine_id = Windows: HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid
             macOS:   IOPlatformUUID from ioreg
             Linux:   /etc/machine-id, else /var/lib/dbus/machine-id
device_hash = "d_" + base32(sha256("smidge-device-v1|" + os + "|" + upper(machine_id))[0..16]).lower()
```

Read the registry with the `windows` crate (or `winreg`), not by spawning `reg.exe`. The raw machine id never leaves the computer. The web install id is `"w_" + base32(16 random bytes)`, created on first load and kept in IndexedDB.

Offline validation (`cia-license::validate(token, device_hash, now, state) -> LicenseState`), identical on desktop and web:

```
1. Split on ".", require prefix "SMG1", decode both parts.
2. Find the public key for payload.kid; unknown kid -> Invalid.
3. Verify the signature; failure -> Invalid.
4. payload.dev != device_hash -> Invalid ("This license belongs to another computer.")
5. Clock check: if now < state.last_seen_utc - 48 h -> NeedsOnline ("Your computer's clock looks wrong.")
6. now > payload.exp -> NeedsOnline ("Smidge needs to check your license. Connect to the internet once.")
7. payload.acc != null and now > payload.acc + 3 days -> Ended ("Your yearly plan ended on <date>.")
8. otherwise Valid { plan, key4, acc }.
state.last_seen_utc = max(state.last_seen_utc, now) is written after every check.
```

`Invalid`, `NeedsOnline` and `Ended` all fall back to the free tier with a banner; nothing is ever locked beyond that.

Refresh: on launch (after the UI is up, never blocking it) and every 24 hours while running, if online, the app calls `POST /api/v1/license/refresh`. A network failure changes nothing. A `revoked`, `ended` or `deactivated` answer deletes the token and shows the reason once.

Desktop files: `<app_data>/license.token` (the token, plain text) and `<app_data>/state.json` (`last_seen_utc`, last refresh time).

## 5.6 Flows

### 5.6.1 Buying from the desktop app (the app unlocks by itself)

```
App                     Site /api/v1               Paddle                    Webhook (site)          Resend
 |                           |                        |                            |                    |
 |-- POST /claims ---------->|  {kind:"desktop", dev, device_name, platform}       |                    |
 |<-- {claim_id, claim_secret, buy_url, expires_at} (2 h)                          |                    |
 |-- open browser: buy_url = https://www.<domain>/buy?claim=clm_...                |                    |
 |                     Browser: /buy shows plans (Paddle.js price preview)          |                    |
 |                     Browser -- POST /checkout {plan, claim_id} -->|              |                    |
 |                           |-- POST /transactions {items:[{price_id}],            |                    |
 |                           |     custom_data:{checkout_id, claim_id}} ---------->|                    |
 |                           |<-- {id: txn_...} -----------------------------------|                    |
 |                     Browser <-- {transaction_id} + Set-Cookie chk=<secret>       |                    |
 |                     Browser: Paddle.Checkout.open({transactionId}); user pays    |                    |
 |                           |                        |-- transaction.completed -->|                    |
 |                           |                        |   (origin "web")           |-- verify, store evt |
 |                           |                        |<-- GET /customers/ctm_ ----|   create entitlement|
 |                           |                        |                            |   + key, fulfil claim
 |                           |                        |                            |-- purchase email -->|
 |-- GET /claims/clm_... (Authorization: Claim <secret>), every 3 s for 10 min, then every 10 s -------->|
 |<-- {status:"fulfilled", product_key, token}   (token minted for the claim's device)                  |
 | save license.token, show "Smidge Pro is on"                                                          |
```

The claim's device becomes the first activation. If the user closes the app before the purchase completes, nothing is lost: the email has the key, and the next launch finds no claim and offers "Enter a key".

### 5.6.2 Activating with a key

```
App                                   Site /api/v1                                  DB
 |-- POST /license/activate {product_key, kind, dev, device_name, platform, app_version} -->|
 |                                     | validate format + checksum; rate limit               |
 |                                     | look up by sha256(key) ------------------------------>|
 |                                     | status must grant access (5.7)                        |
 |                                     | activate_device(ent, kind, dev, ...) (advisory lock) ->|
 |<-- 200 {token, plan, devices_used, devices_max}                                             |
 |<-- 404 {error:"key_not_found"}  |  403 {error:"revoked", reason}  |  409 {error:"device_limit", devices:[{name, platform, last_seen_at}]}
```

On 409 the app shows: "This key is already used on 3 computers: Margaret's PC (last used 3 days ago), ... Remove one in your account, then try again." with [Open my account].

Re-activating the same device (reinstall) returns the existing activation and a fresh token; it never uses a second slot, because the device hash is the same.

### 5.6.3 Web app unlock

Same `POST /license/activate` with `kind:"web"` and the browser's install id. The success page links to `https://app.<domain>/#key=XXXXX-XXXXX-XXXXX-XXXXX`; the fragment never reaches a server, and the web app reads it, activates, then removes it from the address bar with `history.replaceState`. Buying from inside the web app uses the claim flow of 5.6.1 with `kind:"web"`.

### 5.6.4 Refresh

```
App -- POST /license/refresh {token} --> Site
   Site: verify token signature (its own public key), find device row (ent, dev) where not deactivated
         -> not found: 200 {status:"deactivated"}
         -> entitlement revoked: 200 {status:"revoked", reason:"refund"|"chargeback", at}
         -> entitlement ended and acc passed: 200 {status:"ended", at}
         -> else: update last_seen_at, app_version; 200 {status:"ok", token:<new token>}
```

### 5.6.5 Account dashboard sign-in (the only place an email code is used)

```
Browser                           Site /api/v1                                Resend
 |-- POST /auth/otp/send {email} -->|                                            |
 |                                  | rate limit (fail closed)                   |
 |                                  | if a customer with this email exists:      |
 |                                  |   code = 6 random digits (crypto.randomInt)|
 |                                  |   store HMAC(OTP_PEPPER, email:code), 10 min, attempts 0
 |                                  |   send "Your Smidge sign-in code" -------->|
 |<-- 200 {sent:true}  (always the same answer, so it can't be used to test emails)
 |-- POST /auth/otp/verify {email, code} -->|
 |                                  | consume_otp_attempt (atomic, max 5)         |
 |                                  | compare HMAC in constant time               |
 |<-- 200 + Set-Cookie smg_session=<32 random bytes, base64url>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=2592000
 |-- GET /account (cookie) -->|  -> entitlements, devices, masked keys
```

Sessions are rows in `sessions` keyed by `sha256(cookie)`, 30 days sliding, revocable ("Sign out everywhere"). No JWT and no signing secret to misconfigure.

### 5.6.6 Refund

```
Paddle -- adjustment.created {action:"refund", status:"pending_approval"} --> webhook: store, ignore
Paddle -- adjustment.updated {action:"refund", status:"approved", type:"full", transaction_id} --> webhook:
          find entitlement by transaction (purchase txn, or a renewal txn of its subscription)
          status = revoked, revoked_reason = "refund", revoked_at = occurred_at
          insert applied_adjustments(adjustment_id)  (idempotency)
          email "Your refund is done. Smidge Pro is now off." (email_log unique)
App -- next refresh --> {status:"revoked", reason:"refund"} -> delete token, free tier, one-time notice
```

### 5.6.7 Lost key

`POST /api/v1/license/resend {email}` always answers 200. If the email has entitlements, it sends one email listing every key (decrypted from `product_key_enc`). Rate limited 3 per hour per email and 10 per hour per IP. Available from the Activate modal, the site, and the dashboard. No code needed: the key only goes to the purchase email.

## 5.7 Entitlement state machine

```
                      transaction.completed (origin web|api, first purchase)
            (none) -------------------------------------------------------> active
  active  -- subscription status past_due --------------------------------> past_due   (access continues)
  past_due -- subscription status active (payment recovered) --------------> active
  active  -- scheduled cancel (subscription.updated, scheduled_change) ----> active, cancel_at = period end
  active | past_due -- subscription status canceled or paused -------------> ended      (access_until = canceled_at | paused_at)
  ended   -- subscription resumed (status active again) -------------------> active
  any     -- refund approved (full) or chargeback approved ----------------> revoked
  revoked -- chargeback_reverse approved ----------------------------------> state re-derived (lifetime: active; yearly: from subscription)
```

Access rule, the one function every route uses (`apps/site/lib/entitlements.ts: grantsAccess(ent, now)`):

- lifetime: `status = active`
- yearly: `status in (active, past_due)` and `access_until > now`, or `status = ended` and `access_until > now`

For yearly plans `access_until` is set from the subscription projection (5.8): `current_billing_period.ends_at` while active; `current_billing_period.ends_at + 30 days` while past_due (Paddle's dunning window, so a declined card never locks anyone out on the first failure, which was ConvertSave's Stripe bug); `canceled_at` or `paused_at` once ended.

## 5.8 Paddle webhooks

### 5.8.1 Endpoint

`POST /api/v1/webhooks/paddle` (Node runtime, `export const dynamic = "force-dynamic"`):

1. Read the raw body with `await req.text()`. Do not parse JSON before the signature is verified.
2. Verify `Paddle-Signature: ts=<unix>;h1=<hex>[;h1=<hex>...]` with our own function (`apps/site/lib/paddle-signature.ts`), not the SDK's `unmarshal`, because the SDK keeps only the last `h1` (it breaks during secret rotation), compares with `===`, and does not reject future timestamps. Rules: `abs(now - ts) <= 30 s` (future skew at most 5 s); accept if any `h1` equals `HMAC_SHA256(PADDLE_WEBHOOK_SECRET, ts + ":" + rawBody)` compared with `crypto.timingSafeEqual` on equal-length buffers. Failure: 401 with no body detail.
3. `insert into paddle_events (event_id, event_type, occurred_at, payload) ... on conflict (event_id) do nothing`. If the row already existed with status `processed` or `ignored`, return 200 immediately. That makes Paddle's at-least-once delivery harmless.
4. Process (5.8.2) with a 3.5 s time budget (Paddle wants a 200 within 5 s).
5. Success: `status = processed` (or `ignored`), return 200. Any thrown error: `status = error`, `attempts += 1`, `last_error`, return 500 so Paddle retries (live: 60 retries over 3 days). `GET /api/v1/cron/reprocess-webhooks` (Vercel cron every 10 minutes) reprocesses `error` rows and `received` rows older than 2 minutes, up to 20 attempts, then emails `ADMIN_ALERT_EMAIL` once per event.

Ordering: Paddle does not guarantee order. Subscription events never trust the payload's state; they refetch `GET /subscriptions/{id}` and project the current state, so an old event processed late writes the current truth. Transactions and adjustments are idempotent on their own ids. `entitlements.last_event_at` records the newest `occurred_at` applied, for debugging only.

### 5.8.2 Event table

| Event | Condition | Action | Idempotency |
|---|---|---|---|
| `transaction.completed` | `origin` in (`web`, `api`) and an item's price id is `PADDLE_PRICE_ID_LIFETIME` | Upsert customer (fetch `GET /customers/{customer_id}` for the email: it is not in the transaction payload). Create entitlement `plan=lifetime, status=active`, key, link `checkouts`/`claims` from `custom_data.checkout_id`. Send purchase email. | `entitlements.paddle_transaction_id` unique; a unique violation means already done |
| `transaction.completed` | `origin` in (`web`, `api`) and price is `PADDLE_PRICE_ID_YEARLY` | Same, `plan=yearly`; fetch the subscription for `access_until`. | `entitlements.paddle_subscription_id` unique |
| `transaction.completed` | `origin = subscription_recurring` | Refetch the subscription, project `access_until`. No new entitlement, no email. Never infer trials from `price.trial_period` (the property is on the price, so it is present on renewals too; that was the ConvertSave Paddle-branch bug). We have no trials anyway. | projection is idempotent |
| `transaction.completed` | any other origin | `ignored` | n/a |
| `transaction.paid`, `transaction.created`, `transaction.updated`, `transaction.payment_failed`, `transaction.past_due` | | `ignored`. Payment failures are handled through subscription status, after Paddle's retries. | n/a |
| `subscription.created`, `.activated`, `.updated`, `.past_due`, `.paused`, `.resumed`, `.canceled`, `.trialing` | | Refetch subscription. If no entitlement has this subscription id yet, mark `processed` and stop (the `transaction.completed` handler will fetch the subscription itself). Otherwise project: `active` gives status active and `access_until = current_billing_period.ends_at`, `cancel_at = scheduled_change.effective_at` when `scheduled_change.action = cancel`; `past_due` gives status past_due and `access_until = current_billing_period.ends_at + 30 days`; `canceled` gives status ended and `access_until = canceled_at`; `paused` gives status ended and `access_until = paused_at`. A scheduled cancel sends the "Your plan won't renew" email once. | projection; email via `email_log` with kind `cancel_scheduled` and ref = subscription id plus cancel_at |
| `adjustment.created`, `adjustment.updated` | `action = refund`, `status = approved`, `type = full` | Find the entitlement by `transaction_id` (the purchase) or by `subscription_id`. Set `revoked`, reason `refund`. Email "refund done". | `applied_adjustments.adjustment_id` primary key |
| same | `action = refund`, `type = partial` | Audit row only; access unchanged. | same |
| same | `action = chargeback`, `status = approved` | Revoke, reason `chargeback`. No email. | same |
| same | `action = chargeback_reverse`, `status = approved` | Re-derive state (lifetime: active; yearly: refetch subscription and project). | same |
| same | `status` in (`pending_approval`, `rejected`, `reversed`) or `action` in (`credit`, `credit_reverse`, `chargeback_warning`, `chargeback_warning_reverse`) | `ignored` | n/a |
| `customer.updated` | | Update `customers.email` by `paddle_customer_id`. | last write wins |
| anything else | | `ignored` | n/a |

Subscribe the notification destination to exactly: `transaction.completed`, `subscription.*`, `adjustment.created`, `adjustment.updated`, `customer.updated`.

### 5.8.3 Checkout creation

`POST /api/v1/checkout {plan: "lifetime" | "yearly", claim_id?: string}`:

1. Validate `claim_id` exists, is pending and not expired (if given).
2. Insert `checkouts (id = "chk_" + ULID, plan, paddle_price_id, claim_id, secret_hash)`.
3. `POST https://(sandbox-)api.paddle.com/transactions` with `items: [{ price_id, quantity: 1 }]`, `custom_data: { checkout_id, claim_id }`, `collection_mode: "automatic"`. Custom data set on the server cannot be edited by the buyer, unlike `customData` passed to Paddle.js in the browser.
4. Store `paddle_transaction_id` on the checkout; return `{ transaction_id }` and set cookie `smg_chk=<checkout secret>; HttpOnly; Secure; SameSite=Lax; Max-Age=7200; Path=/success`.
5. The browser opens `Paddle.Checkout.open({ transactionId, settings: { displayMode: "overlay", allowLogout: false, successUrl: "https://www.<domain>/success?c=<checkout_id>" } })`.

`/success?c=chk_...` polls `GET /api/v1/checkout/{id}` (requires the `smg_chk` cookie) until the webhook has fulfilled it, then shows the key, "Smidge on your computer unlocks by itself if you started from the app", [Download Smidge] and [Use Smidge in your browser] (the `#key=` link). The key is shown there for 2 hours after purchase only.

## 5.9 Database schema

Migrations live in `apps/site/supabase/migrations/NNNN_name.sql` and are applied with `supabase db push` (dev) or by Hunter in the SQL editor (prod) before the code that needs them is deployed. There is no hand-maintained schema file: `supabase db dump --schema-only > apps/site/supabase/schema.sql` regenerates it, and CI fails if it is stale. (The Paddle branch renamed columns in a `CREATE TABLE IF NOT EXISTS` file that never touched production; a migration-only workflow prevents that.)

`0001_init.sql`:

```sql
create extension if not exists pgcrypto;

create type plan_t as enum ('lifetime', 'yearly');
create type ent_status_t as enum ('active', 'past_due', 'ended', 'revoked');

create table customers (
  id uuid primary key default gen_random_uuid(),
  paddle_customer_id text unique,
  email text not null,                          -- lowercased, trimmed
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index customers_email_idx on customers (email);

create table entitlements (
  id text primary key,                          -- 'ent_' + ULID
  customer_id uuid not null references customers(id),
  plan plan_t not null,
  status ent_status_t not null,
  product_key_hash bytea not null unique,
  product_key_enc text not null,
  key4 text not null,
  max_devices int not null default 3,
  max_web int not null default 3,
  paddle_transaction_id text unique,            -- purchase transaction
  paddle_subscription_id text unique,           -- yearly only
  paddle_price_id text not null,
  access_until timestamptz,                     -- null for lifetime
  cancel_at timestamptz,
  revoked_at timestamptz,
  revoked_reason text check (revoked_reason in ('refund', 'chargeback', 'manual')),
  last_event_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  constraint yearly_has_sub check ((plan = 'yearly') = (paddle_subscription_id is not null))
);
create index entitlements_customer_idx on entitlements (customer_id);

create table devices (
  id uuid primary key default gen_random_uuid(),
  entitlement_id text not null references entitlements(id) on delete cascade,
  kind text not null check (kind in ('desktop', 'web')),
  device_hash text not null,
  device_name text,
  platform text check (platform in ('windows', 'macos', 'linux', 'web')),
  app_version text,
  activated_at timestamptz not null default now(),
  last_seen_at timestamptz not null default now(),
  deactivated_at timestamptz,
  deactivated_by text check (deactivated_by in ('device', 'dashboard', 'idle', 'evicted', 'support'))
);
create unique index devices_active_uq on devices (entitlement_id, device_hash) where deactivated_at is null;

create table claims (
  id text primary key,                          -- 'clm_' + ULID
  secret_hash bytea not null,
  kind text not null check (kind in ('desktop', 'web')),
  device_hash text not null,
  device_name text,
  platform text,
  entitlement_id text references entitlements(id),
  status text not null default 'pending' check (status in ('pending', 'fulfilled', 'expired')),
  created_at timestamptz not null default now(),
  expires_at timestamptz not null
);

create table checkouts (
  id text primary key,                          -- 'chk_' + ULID
  secret_hash bytea not null,
  plan plan_t not null,
  paddle_price_id text not null,
  paddle_transaction_id text unique,
  claim_id text references claims(id),
  entitlement_id text references entitlements(id),
  created_at timestamptz not null default now()
);

create table paddle_events (
  event_id text primary key,
  event_type text not null,
  occurred_at timestamptz not null,
  payload jsonb not null,
  received_at timestamptz not null default now(),
  status text not null default 'received' check (status in ('received', 'processed', 'ignored', 'error')),
  attempts int not null default 0,
  last_error text,
  processed_at timestamptz
);
create index paddle_events_status_idx on paddle_events (status, received_at);

create table applied_adjustments (
  adjustment_id text primary key,
  entitlement_id text not null references entitlements(id),
  action text not null,
  applied_at timestamptz not null default now()
);

create table otp_codes (
  email text primary key,
  code_hmac bytea not null,                     -- HMAC-SHA256(OTP_PEPPER, email || ':' || code)
  sent_at timestamptz not null,
  expires_at timestamptz not null,
  attempts int not null default 0,
  used_at timestamptz
);

create table sessions (
  id_hash bytea primary key,                    -- sha256(cookie value)
  email text not null,
  created_at timestamptz not null default now(),
  last_used_at timestamptz not null default now(),
  expires_at timestamptz not null,
  revoked_at timestamptz,
  user_agent text
);

create table email_log (
  id uuid primary key default gen_random_uuid(),
  kind text not null,
  ref text not null,
  to_email text not null,
  resend_id text,
  sent_at timestamptz not null default now(),
  unique (kind, ref)
);

create table audit_log (
  id bigserial primary key,
  at timestamptz not null default now(),
  actor text not null,                          -- 'webhook', 'api', 'dashboard', 'support', 'cron'
  action text not null,
  entitlement_id text,
  detail jsonb
);

create table rate_limits (
  key text not null,
  window_start timestamptz not null,
  count int not null default 0,
  primary key (key, window_start)
);

alter table customers enable row level security;
alter table entitlements enable row level security;
alter table devices enable row level security;
alter table claims enable row level security;
alter table checkouts enable row level security;
alter table paddle_events enable row level security;
alter table applied_adjustments enable row level security;
alter table otp_codes enable row level security;
alter table sessions enable row level security;
alter table email_log enable row level security;
alter table audit_log enable row level security;
alter table rate_limits enable row level security;
-- No policies: only the service role (which bypasses RLS) can read or write.
```

`0002_functions.sql` defines, each `security definer`, `revoke all ... from public`, `grant execute ... to service_role`:

- `activate_device(p_ent text, p_kind text, p_hash text, p_name text, p_platform text, p_version text) returns jsonb`: takes `pg_advisory_xact_lock(hashtextextended(p_ent, 0))`; if an active row exists for `(p_ent, p_hash)`, update `last_seen_at`, `app_version` and return `{ok: true, existing: true, used, max}`; if `p_kind = 'web'` and active web rows >= `max_web`, deactivate the one with the oldest `last_seen_at` (`deactivated_by = 'evicted'`); if `p_kind = 'desktop'` and active desktop rows >= `max_devices`, return `{ok: false, reason: 'limit', devices: [...]}`; otherwise insert and return `{ok: true, used, max}`. Copy the locking pattern from ConvertSave's `activate_device`, minus its non-atomic fallback path in `lib/activation.ts`: if the function is missing, the request fails.
- `consume_otp_attempt(p_email text, p_max int) returns table(code_hmac bytea, expires_at timestamptz)`: the ConvertSave function, adapted to `code_hmac` and `used_at`.
- `rate_limit_hit(p_key text, p_window_seconds int, p_max int) returns boolean`: the ConvertSave function unchanged. The calling code fails closed: if the RPC errors, the request gets 503 (ConvertSave's version fails open).

## 5.10 API routes

All under `https://www.<domain>/api/v1`. Request and response bodies are zod schemas in `packages/api-types`. Errors are `{ error: "<snake_case_code>", message: "<user-facing sentence>" }`. CORS: `Access-Control-Allow-Origin: https://app.<domain>` on the routes the web app calls (marked W); the desktop app is not a browser and needs no CORS.

| Method | Path | Called by | Auth | Body → response |
|---|---|---|---|---|
| POST | `/claims` | app (W) | none | `{kind, device_hash, device_name, platform}` → `{claim_id, claim_secret, buy_url, expires_at}` |
| GET | `/claims/{id}` | app (W) | `Authorization: Claim <secret>` | → `{status:"pending"}` / `{status:"fulfilled", product_key, token, plan}` / `{status:"expired"}` |
| POST | `/checkout` | site browser | none | `{plan, claim_id?}` → `{transaction_id}` + `smg_chk` cookie |
| GET | `/checkout/{id}` | site browser | `smg_chk` cookie | → `{status:"pending"}` / `{status:"fulfilled", product_key, plan}` |
| POST | `/license/activate` | app (W) | none | `{product_key, kind, device_hash, device_name, platform, app_version}` → `{token, plan, devices_used, devices_max}`; 404 `key_not_found`, 403 `revoked`/`ended`, 409 `device_limit` |
| POST | `/license/refresh` | app (W) | token | `{token}` → `{status:"ok", token}` / `{status: "revoked", reason, at}`, or the same with status `"ended"` or `"deactivated"` |
| POST | `/license/deactivate` | app (W) | token | `{token}` → `{ok:true}` |
| POST | `/license/resend` | app (W), site | none | `{email}` → `{ok:true}` always |
| POST | `/auth/otp/send` | site | none | `{email}` → `{sent:true}` always |
| POST | `/auth/otp/verify` | site | none | `{email, code}` → `{ok:true}` + `smg_session` cookie; 400 `invalid_code` |
| POST | `/auth/logout` | site | session | `{everywhere?: boolean}` → `{ok:true}` |
| GET | `/account` | site | session | → `{email, entitlements:[{id, plan, status, key_masked, key4, created_at, access_until, cancel_at, devices:[...], web_count}]}` |
| POST | `/account/reveal-key` | site | session | `{entitlement_id}` → `{product_key}` (audit logged) |
| POST | `/account/devices/{id}/deactivate` | site | session | → `{ok:true}`; max 10 per entitlement per 30 days, then 429 "Contact support" |
| POST | `/account/web/signout` | site | session | `{entitlement_id}` → deactivates every web row |
| POST | `/account/portal` | site | session | `{entitlement_id}` → `{url}` from Paddle `POST /customers/{id}/portal-sessions` (never cached; the links are temporary) |
| POST | `/webhooks/paddle` | Paddle | signature | 5.8 |
| GET | `/updates/{channel}/{target}/{arch}/{current_version}` | desktop updater | none | Tauri updater JSON, or 204 when up to date (6.3) |
| GET | `/cron/reprocess-webhooks` | Vercel cron, every 10 min | `Authorization: Bearer CRON_SECRET` | 5.8.1 |
| GET | `/cron/housekeeping` | Vercel cron, hourly | same | expire claims, delete used/expired OTP rows, delete rate-limit windows older than a day, mark desktop devices unseen for 180 days `idle` |
| GET | `/health` | anyone | none | `{ok:true, version}` |

Every POST from a browser also checks `Origin` against `NEXT_PUBLIC_SITE_URL` / `NEXT_PUBLIC_APP_URL` (CSRF), in addition to `SameSite=Lax` cookies.

Rate limits (all fail closed, keys use `sha256(IP_HASH_SALT + ip)`):

| Route | Limits |
|---|---|
| POST /claims | 20 per hour per IP |
| GET /claims/{id} | 1,500 per claim (2 h at one per 3 s, with margin) |
| POST /checkout | 30 per hour per IP |
| POST /license/activate | 10 per hour per IP; 30 per day per key |
| POST /license/refresh | 60 per hour per device |
| POST /license/resend | 3 per hour per email; 10 per hour per IP |
| POST /auth/otp/send | 3 per 15 min per email (plus 60 s resend cooldown); 20 per hour per IP |
| POST /auth/otp/verify | 5 attempts per code; 30 per hour per IP |

## 5.11 Email (Resend)

From `EMAIL_FROM` on a verified subdomain (`mail.<domain>`, SPF and DKIM via Resend), reply-to `EMAIL_REPLY_TO`. Templates in `apps/site/emails/*.tsx` (React Email). Every send goes through `sendOnce(kind, ref, to, template)`, which inserts into `email_log` first and skips if `(kind, ref)` exists, so webhook retries never send twice (the Paddle branch re-sent the key on every renewal).

| kind | ref | When | Content |
|---|---|---|---|
| `purchase_key` | entitlement id | first purchase | key, "how to activate" for desktop and web, download links |
| `otp` | email + sent_at | dashboard sign-in | 6-digit code, valid 10 minutes |
| `key_resend` | email + hour | lost key | all keys for the email |
| `cancel_scheduled` | subscription id + cancel_at | scheduled cancel | "Your plan won't renew. Pro stays on until <date>." |
| `access_ended` | entitlement id + access_until | yearly ended | how to renew |
| `refund_done` | adjustment id | full refund | Pro is off; free tier still works |

## 5.12 Account dashboard

`www.<domain>/account`, after sign-in:

- One card per entitlement: "Smidge Pro · Lifetime · bought 2 Oct 2026", or "Smidge Pro · Yearly · renews 2 Oct 2027" / "ends 2 Oct 2027" / "Payment problem: update your card in Manage billing" (past_due) / "Refunded on 5 Oct 2026".
- Product key masked (`XXXXX-XXXXX-XXXXX-7KQ2P`) with [Show], [Copy], [Email it to me].
- Computers: name, platform, last used, [Remove]. "2 of 3 computers in use."
- Browsers: "Used in 2 browsers" [Sign out all browsers].
- [Manage billing] (Paddle portal: card, invoices, cancel), [Download Smidge] (Windows, macOS, Linux links from the stable channel), [Sign out], [Sign out everywhere].
- No profile, no settings, no password, nothing else.

## 5.13 Environment variables

Site (Vercel project `smidge-site`, root `apps/site`). `apps/site/lib/env.ts` validates all of them with zod at startup and throws on anything missing, placeholder-like (the ConvertSave `lib/jwt.ts` marker list) or inconsistent, for example `PADDLE_ENV=production` with a key that does not start `pdl_live_`. There is no fallback value for any secret (the Paddle branch silently fell back to a dummy key and sandbox).

| Variable | Example / format | Notes |
|---|---|---|
| `NEXT_PUBLIC_SITE_URL` | `https://www.<domain>` | |
| `NEXT_PUBLIC_APP_URL` | `https://app.<domain>` | |
| `SUPABASE_URL` | `https://<ref>.supabase.co` | |
| `SUPABASE_SERVICE_ROLE_KEY` | | server only |
| `PADDLE_ENV` | `sandbox` or `production` | selects `sandbox-api.paddle.com` or `api.paddle.com` |
| `PADDLE_API_KEY` | `pdl_sdbx_apikey_...` / `pdl_live_apikey_...` | |
| `PADDLE_WEBHOOK_SECRET` | `pdl_ntfset_...` | notification destination secret |
| `PADDLE_PRICE_ID_LIFETIME` | `pri_...` | one-time price |
| `PADDLE_PRICE_ID_YEARLY` | `pri_...` | yearly price, no trial period |
| `NEXT_PUBLIC_PADDLE_ENV` | `sandbox` / `production` | for Paddle.js |
| `NEXT_PUBLIC_PADDLE_CLIENT_TOKEN` | `test_...` / `live_...` | |
| `RESEND_API_KEY` | `re_...` | |
| `EMAIL_FROM` | `Smidge <hello@mail.<domain>>` | |
| `EMAIL_REPLY_TO` | `support@<domain>` | |
| `LICENSE_SIGNING_KEY` | base64 of the 32-byte Ed25519 seed | generate with `cargo xtask keygen license` |
| `LICENSE_SIGNING_KID` | `2026-10` | must exist in `cia-license/src/keys.rs` |
| `KEY_ENCRYPTION_KEY` | base64 of 32 random bytes | AES-256-GCM for stored product keys |
| `OTP_PEPPER` | base64 of 32 random bytes | |
| `IP_HASH_SALT` | base64 of 16 random bytes | |
| `CRON_SECRET` | random | Vercel cron auth |
| `ADMIN_ALERT_EMAIL` | Hunter's address | dead-letter webhook alerts |

Desktop build-time (`option_env!`, set in the release workflow): `CIA_API_BASE` (default `https://www.<domain>/api/v1`; dev builds use the preview URL), `CIA_BUILD_CHANNEL` (`stable`/`beta`/`dev`). Updater public key in `tauri.conf.json`. No secrets.

Web build-time (Vite): `VITE_API_BASE`, `VITE_SITE_URL`.

CI secrets (GitHub, private monorepo): `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_ENDPOINT`, `AZURE_CODE_SIGNING_NAME`, `AZURE_CERT_PROFILE_NAME`, `DOWNLOADS_REPO_TOKEN` (fine-grained token: contents write on the Downloads repo only), `CLOUDFLARE_API_TOKEN`, `CLOUDFLARE_ACCOUNT_ID`, `VERCEL_TOKEN`, `VERCEL_ORG_ID`, `VERCEL_PROJECT_ID`. Libraries repo: `LIBRARIES_MANIFEST_KEY` (Ed25519 seed for signing the FFmpeg manifest) and nothing else.

## 5.14 ConvertSave Paddle-branch bugs and the rule that prevents each

| Bug in `feature/switch-stripe-to-paddle` | Rule here |
|---|---|
| Renewals treated as trials (`price.trial_period != null`), then the trial-abuse check cancelled paying subscribers | No trials. First purchase vs renewal decided by `transaction.origin` only |
| Schema renamed columns in a `CREATE TABLE IF NOT EXISTS` file; production never changed | Migrations only; generated schema dump checked in CI |
| Stale base reverted security hardening | Fresh project; OTP, rate limit and activation hardening are in `0001`/`0002` from the start, with no fail-open fallbacks |
| Instant deactivation on `payment_failed` / `past_due` | Payment failures do nothing; `past_due` keeps access for Paddle's 30-day dunning window; only `canceled`/`paused` end access |
| No webhook idempotency, duplicate emails on retry | `paddle_events.event_id` primary key, entity-level unique keys, `email_log (kind, ref)` unique |
| No signature timestamp check (replay) | 30 s window plus event-id dedupe |
| `lib/paddle.ts` fell back to a dummy key and sandbox | `env.ts` fails closed and cross-checks environment against key prefixes |
| Refund handled on `adjustment.created` (usually still `pending_approval`) and only for lifetime | Act on `status = approved` from `.created` or `.updated`; refunds revoke any plan |
| Product key re-emailed on every renewal | Purchase email keyed on entitlement id, sent once |
| Trial OTP gate bypassable by URL parameter | No trial, no gate to bypass; checkout data set server-side |
| Every webhook error swallowed with 200 | Errors return 500 so Paddle retries, plus a cron dead-letter path |

---
# 6. Distribution

## 6.1 Repositories

| Repo | Visibility | Purpose |
|---|---|---|
| `Hunter-Boone/CompressItAll` | private | The monorepo: engine, desktop app, web app, site, tests, workflows. |
| `Hunter-Boone/Smidge-Downloads` | public | Desktop installers and updater artifacts as GitHub Releases, `channels/stable.json` and `channels/beta.json`, and a user-facing README with download links and system requirements. Issues disabled (support is by email). The equivalent of ConvertSave-Support. |
| `Hunter-Boone/Smidge-Libraries` | public | Build workflows for the LGPL FFmpeg that Smidge downloads, `BUILD_CONFIG.txt`, and releases containing binaries, the matching source tarball and a signed manifest. The equivalent of ConvertSave-Libraries. |

The engine stays private. Nothing in it is required to be public: MPL-2.0 components (Symphonia, mediabunny) are used unmodified and their source is upstream; LGPL applies only to FFmpeg, whose exact source is published in Smidge-Libraries.

Why a new Libraries repo instead of new tags in ConvertSave-Libraries: Smidge needs hardware-encoder flags ConvertSave's build lacks, it needs immutable pinned releases with a signed manifest (ConvertSave's app downloads the moving `ffmpeg-latest` tag), and public download URLs should carry this product's name. ConvertSave can move to the new repo later if Hunter wants one build for both.

## 6.2 Desktop release pipeline

Workflows in `CompressItAll/.github/workflows/`:

**`test.yml`**: runs on every PR and push to `main`; also `workflow_call`-able (7.7).

**`release-desktop.yml`**: on tag `v*.*.*` (and `v*.*.*-beta.N`):

1. `needs: test` (calls `test.yml`). ConvertSave's release does not run tests; this one cannot publish a build that fails them.
2. Build matrix:
   - `windows-latest`: `x86_64-pc-windows-msvc`, NSIS installer (per-user install, no admin prompt) and MSI.
   - `macos-latest`: `universal-apple-darwin`, one `.dmg` and one updater `.app.tar.gz` (ConvertSave ships two separate Intel/ARM builds; one universal file is simpler for users and for the updater manifest).
   - `ubuntu-22.04`: AppImage and `.deb`. Built on 22.04, not `latest`, so the AppImage runs on older glibc (2.35+).
3. Windows signing happens inside the Tauri bundler through `bundle.windows.signCommand` calling `trusted-signing-cli` with the Azure Trusted Signing secrets. Every exe and the installer are signed before Tauri computes the updater signature, so ConvertSave's "re-sign, then regenerate .sig" step disappears.
4. macOS signing and notarisation via Tauri's built-in support (`APPLE_CERTIFICATE`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`), with stapling. Hardened runtime on; entitlements: none beyond the defaults (no JIT, no camera, no network server).
5. Collect assets named `Smidge_<version>_<Windows_x64|macOS_universal|Linux_x86_64>.<ext>` plus `.sig` files, `SHA256SUMS.txt`, `NOTICE.md` (from `licenses-data`), and `release.json` (version, date, notes from `CHANGELOG.md`, per-platform URL and signature).
6. Create a **draft** release in Smidge-Downloads with `DOWNLOADS_REPO_TOKEN`. Draft releases are invisible to users and to the updater.

**Lab smoke (manual, before promotion):** `python3 tools/lab/cia_lab.py smoke <version> {linux|windows|mac}` downloads the draft assets with `gh release download`, installs them on the lab machine, launches the app, and runs the native smoke spec (7.5). Results land in `TestLab/results/`.

**`promote.yml`** (`workflow_dispatch`, inputs `version`, `channel` = stable|beta, `rollout_percent` default 100): publishes the draft (beta: as prerelease), then commits `channels/<channel>.json` to Smidge-Downloads `main`:

```json
{
  "version": "1.2.0",
  "pub_date": "2026-11-20T15:00:00Z",
  "notes": "Faster PDF compression. Fixed sideways videos from some Android phones.",
  "rollout_percent": 25,
  "blocked_versions": ["1.1.9"],
  "platforms": {
    "windows-x86_64": { "url": "https://github.com/Hunter-Boone/Smidge-Downloads/releases/download/v1.2.0/Smidge_1.2.0_Windows_x64-setup.exe", "signature": "<contents of .sig>" },
    "darwin-aarch64": { "url": ".../Smidge_1.2.0_macOS_universal.app.tar.gz", "signature": "..." },
    "darwin-x86_64":  { "url": ".../Smidge_1.2.0_macOS_universal.app.tar.gz", "signature": "..." },
    "linux-x86_64":   { "url": ".../Smidge_1.2.0_Linux_x86_64.AppImage", "signature": "..." }
  }
}
```

Raising the rollout is another `promote.yml` run with a higher percentage. A bad release is pulled by setting `rollout_percent` to 0 (or promoting the previous version with a higher patch number).

## 6.3 Updater

Tauri updater plugin, Ed25519 updater key (separate from the license key). The Rust side builds the updater at runtime so it can pick the channel:

```rust
app.updater_builder()
    .endpoints(vec![
        format!("https://www.<domain>/api/v1/updates/{channel}/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}").parse()?,
        format!("https://raw.githubusercontent.com/Hunter-Boone/Smidge-Downloads/main/channels/{channel}.tauri.json").parse()?,
    ])?
    .header("X-Smidge-Install", install_id)?
    .build()?
```

- Primary endpoint: the site route reads `channels/<channel>.json` from the Downloads repo (cached 60 s). It returns 204 if the caller is already current. A caller whose version is in `blocked_versions` always gets the update. Everyone else gets it only if `sha256(install_id) mod 100 < rollout_percent`. The install id is a random value created at first launch, unrelated to the license device hash.
- Fallback endpoint: a plain Tauri `latest.json` (`channels/<channel>.tauri.json`, written by `promote.yml` only when `rollout_percent` is 100), used if the site is down.
- Check at launch and every 6 hours. The update downloads in the background; the app shows "Update ready. Restart Smidge to finish." with [Restart now] and [Later], and never restarts during a job.
- Linux: only the AppImage self-updates. The `.deb` shows "A new version is available" with a link to the Downloads page.

## 6.4 FFmpeg distribution (Smidge-Libraries)

Copy `ConvertSave-Libraries/.github/workflows/build-ffmpeg.yml` and `ffmpeg/BUILD_CONFIG.txt`, then change:

- Pin one FFmpeg release (the newest stable release when M5 starts) by tag and by tarball SHA-256.
- Configure as ConvertSave (`--enable-version3`, static, libvpx, libaom, libwebp, libopus, libvorbis, libmp3lame, libzimg, zlib, lzma), plus:
  - all platforms: `--enable-libdav1d` (BSD-2, fast AV1 decode), `--disable-doc --disable-ffplay`;
  - Windows: `--enable-ffnvcodec --enable-nvenc --enable-amf --enable-libvpl --enable-d3d11va --enable-mediafoundation`;
  - Linux: `--enable-vaapi --enable-ffnvcodec --enable-nvenc --enable-libvpl`;
  - macOS: as ConvertSave (VideoToolbox, AudioToolbox).
  NVENC, AMF and libvpl are loaded from the user's driver at runtime through MIT-licensed headers; none of them needs `--enable-nonfree`.
- A CI step that fails the build if `ffmpeg -version` shows `--enable-gpl` or `--enable-nonfree`, or if `ffmpeg -L` does not print the LGPL notice.
- Release tag `ffmpeg-<version>-r<N>`, never moved or reused. Assets: `ffmpeg-<version>-<windows-x86_64|macos-aarch64|macos-x86_64|linux-x86_64>.zip` (each with `ffmpeg`, `ffprobe`, `LICENSE.md`, `BUILD_CONFIG.txt`), `ffmpeg-<version>-source.tar.xz`, `manifest.json`, `manifest.json.sig`.
- macOS binaries get the linker's ad-hoc signature (required on Apple Silicon). Smidge downloads them with its own HTTP client, which sets no quarantine attribute, so Gatekeeper does not assess them; no Developer ID secrets are needed in the public repo.

`manifest.json`:

```json
{
  "schema": 1,
  "name": "ffmpeg",
  "version": "7.1.2",
  "revision": 1,
  "license": "LGPL-3.0-or-later",
  "source": { "url": "https://github.com/Hunter-Boone/Smidge-Libraries/releases/download/ffmpeg-7.1.2-r1/ffmpeg-7.1.2-source.tar.xz", "sha256": "..." },
  "assets": {
    "windows-x86_64": { "url": ".../ffmpeg-7.1.2-windows-x86_64.zip", "sha256": "...", "bytes": 35651584, "unpacked_bytes": 94371840 },
    "macos-aarch64":  { "url": "...", "sha256": "...", "bytes": 0, "unpacked_bytes": 0 },
    "macos-x86_64":   { "url": "...", "sha256": "...", "bytes": 0, "unpacked_bytes": 0 },
    "linux-x86_64":   { "url": "...", "sha256": "...", "bytes": 0, "unpacked_bytes": 0 }
  }
}
```

`manifest.json.sig` is an Ed25519 signature over the exact manifest bytes with `LIBRARIES_MANIFEST_KEY`. The app pins the manifest URL for its FFmpeg version in `crates/cia-ffmpeg/src/pinned.rs` together with the public key; it verifies the signature, then the zip's SHA-256, then unpacks to `tools/ffmpeg/<version>/`. A new FFmpeg reaches users only through an app release that changes the pin, so a broken upstream build can never reach every user at once.

## 6.5 Web app and site hosting

**Web app on Cloudflare Pages** (`app.<domain>`). Reasons: `_headers` gives COOP/COEP/CSP per path with no server code; static assets are served from Cloudflare's edge with no bandwidth charge, which matters when every new visitor downloads several megabytes of WASM; the 25 MiB per-file limit is well above the WASM size. Workflow `web.yml`: on PR, build and run the web Playwright suite; on push to `main`, deploy a preview with `wrangler pages deploy apps/web/dist --branch main`; on tag `web-v*`, deploy to production. The web app is versioned separately from the desktop app and shows its version in About.

**Site on Vercel** (`www.<domain>`), the same as ConvertSave-Website, because it is a Next.js app with API routes and cron. Workflow `site.yml` deploys with the Vercel CLI and `VERCEL_TOKEN` (Git-triggered deploys are not used, as with ConvertSave): previews on PRs; production only through a GitHub environment named `production` that requires Hunter's approval. Crons in `apps/site/vercel.json`.

DNS: `<domain>` redirects to `www`, `www` → Vercel, `app` → Cloudflare Pages, `mail` → Resend records.

**Development servers on the VM** (from `/home/hunter/work/CompressItAll`): web app `npm run dev -w apps/web` on port 5181, site `npm run dev -w apps/site` on 3031. Any dev server left running for others to look at MUST be registered on http://links.floo.network with the `register-webapp` skill (names "Smidge web app (dev)" and "Smidge site (dev)"), and removed from the board when retired.

## 6.6 What this does better than ConvertSave, and why

| ConvertSave today | Smidge | Why it matters |
|---|---|---|
| Release workflow publishes without running tests | `release-desktop.yml` needs `test.yml` | A failing build cannot ship |
| Release goes straight to the public "latest" | Draft, lab smoke, then `promote.yml` | A human sees the real installers before customers do |
| Updater reads a static `latest.json` | Site route with staged rollout, blocked versions and a static fallback | A bad update can be stopped at 5 percent instead of hitting everyone |
| Windows re-signs after bundling, then regenerates signatures | `signCommand` signs during bundling | One step fewer to get wrong; updater signature always matches the signed binary |
| Two macOS builds | One universal build | One download for users, one manifest entry |
| App downloads moving `ffmpeg-latest` | Pinned release, SHA-256, signed manifest | A bad FFmpeg build cannot reach users without an app release |
| CI tests against distro FFmpeg (GPL, has x264) | CI uses the same LGPL build users get | Tests see the same missing encoders users do |
| AES key in the app binary decrypts and could mint licenses | Ed25519 public key only | Extracting the app's key gives nothing |
| Hand-written license texts in `licenses.ts` | Generated from Cargo and npm metadata, CI fails if stale | The licences page stays complete without manual work |
| Ubuntu `latest` builds | Ubuntu 22.04 builds | AppImage runs on more Linux installs |

## 6.7 Code signing and notarisation needs

- Windows: Azure Trusted Signing account and certificate profile (ConvertSave has one; whether Smidge reuses it depends on the seller entity, section 9).
- macOS: Apple Developer ID Application certificate and notarisation credentials (app-specific password) for the same team.
- Linux: none required. The AppImage is covered by the updater signature.
- Tauri updater key pair: new for Smidge (`npx tauri signer generate`), private key in CI secrets, public key in `tauri.conf.json`.
- License signing key and Libraries manifest key: new Ed25519 pairs generated by `cargo xtask keygen`, private halves only in Vercel / the Libraries repo secrets, stored offline by Hunter as a backup.

---
# 7. Testing

The bar for "done": a milestone counts only when its tests pass in CI on all three operating systems and, for desktop milestones, on the lab VMs. Never describe something as working without having run it.

## 7.1 Rust unit tests

Each crate has `#[cfg(test)]` modules plus `tests/` integration tests. Required coverage:

- `cia-core`: preset parsing and schema validation against `presets.schema.json`; `mime_base64` budget math (Gmail gives exactly 18,196,153); per-message water-filling (sums never exceed B, items never below their floor, redistribution terminates); first-fit-decreasing split suggestions; output naming (collisions, emoji, Windows reserved names, long paths); allowance rolling window; every `copy.rs` message for every outcome variant (snapshot tests with `insta`, MIT/Apache).
- `cia-image`: quality search converges within 8 encodes on 20 reference images; never returns a size at or over budget; downscale never upscales; EXIF orientation 1 to 8 produces upright pixels; alpha never routed to JPEG; candidate choice prefers JPEG on score ties; Smaller mode keeps originals that don't shrink 5 percent.
- `cia-mozjpeg`, `cia-webp`: round trip encode/decode at several qualities and sizes, error path (zero-size image) returns an error instead of crashing on native.
- `cia-video-plan`: budget, overhead, audio ladder (15 percent cap), resolution ladder (portrait and landscape, never upscale, even dimensions), refusal max-duration (the computed duration fits; one second more does not), retry scaling, hardware vs software margins. These are table-driven tests with expected numbers written out.
- `cia-ffmpeg`: argument builders produce exactly the commands in 3.5.7 for each encoder (snapshot), progress parser, watchdog (fake process that stalls), path quoting with spaces, emoji and CJK names (real execution on all three OS in CI).
- `cia-pdf`, `cia-office`, `cia-archive`, `cia-audio`: round trips and the verification functions themselves (feed them deliberately broken files and confirm they fail).
- `cia-license`: product key generation distribution (chi-square over 100,000 keys), checksum catches every single-character substitution, shared test vectors with TypeScript, token verify with valid, tampered, wrong-kid, wrong-device, expired, rolled-back-clock and ended cases.

## 7.2 Fixtures

`cargo xtask fixtures` generates `fixtures/synth/` deterministically:

- Images: photographic noise-plus-gradient images at 1, 12 and 48 MP; a 4K UI screenshot rendering (text, flat colour); a transparent logo; a transparent photo cut-out; 16-bit PNG; images with each EXIF orientation; a 15 MB animated GIF.
- Video (requires FFmpeg): `testsrc2` plus `sine` at 10 s, 2 min and 15 min; 480p, 720p, 1080p, 1440p and 2160p; 30 and 60 fps; landscape and portrait; with and without audio; a two-audio-track MKV; a rotation-tagged MP4 (`-display_rotation 90`); an HLG and a PQ HDR clip (`zscale` to BT.2020 with the right transfer tags); a VFR clip (`-fps_mode vfr` from variable `setpts`).
- Audio: 10 s, 5 min and 70 min sine-and-noise WAV, FLAC, MP3, Ogg Vorbis.
- PDF: image-heavy PDF built with `pdf-writer` from the synthetic photos (40 MB), vector-only PDF, encrypted PDF.
- Office: docx and pptx built from templates with embedded synthetic photos (and an MP4 in the pptx).
- Archives: zip and 7z of the synthetic photos; a zip containing a pptx.

Real-world fixtures in `fixtures/real/` (gitignored). The canonical copy lives at `/mnt/nas/SharedFolder2/projects/CompressItAll/fixtures/real/` on the NAS, `fixtures/README.md` records where each came from, and `cargo xtask fixtures sync` copies them to clones and lab VMs. Required set: iPhone HDR HEVC `.mov` (Dolby Vision 8.4), iPhone HEIC photos (portrait and landscape), Android VFR MP4, OBS MKV with two audio tracks, a 4K60 game capture, a screen recording over one hour, a ProRes `.mov`, VP9 WebM, AV1 MP4, a truncated MP4, a file named `Grandkids 🎂 誕生日 (final).mov`, a 60 MB PPTX with photos and a video, a scanned 40 MB PDF, a password-protected PDF and DOCX, a 23-photo mixed HEIC/JPEG folder. ConvertSave's `_sample_files/` may be reused where it has matches; record that in the README.

## 7.3 Engine matrix

`cia matrix --fixtures fixtures --presets all --out target/matrix/<UTC stamp>` runs every fixture against every preset (and Smaller mode) with the desktop engine and writes `matrix.json`, `matrix.md` (a pass/fail table) and `junit.xml` (one test case per row, so the TestLab dashboard can show it).

A row passes when one of these holds:

1. Outcome `Fitted`, and the matrix runner re-verifies the written file with code independent of the engine's own verifier (`ffprobe` and a full `ffmpeg -f null` decode for video and audio, `image` decode for images, a fresh lopdf load for PDFs, a fresh zip open with CRC checks for documents and archives) and every 3.11 check passes.
2. Outcome `KeptOriginal`, and the original is under the limit (or Smaller mode found nothing 5 percent smaller).
3. Outcome `Refused`, and `fixtures/expectations.toml` lists that fixture-preset pair as an expected refusal, or the refusal is `TooLongForLimit` and the matrix's own oracle (3.5.5 formula) agrees within 10 percent.

Any `Fitted` row whose file is at or over the limit is an **honesty violation**: the runner prints it first, in red, and exits with code 2. CI treats code 2 as a release blocker regardless of anything else.

`expectations.toml` also states minimum quality labels for important rows, for example the OBS clip on Discord Free must be at least Good. Nightly runs compare SSIMULACRA2 (images) and bpp (video) per row with the previous run and flag drops of more than 3 points or 10 percent.

Matrix sizes: `--smoke` (synthetic images, PDFs, archives, 10 s videos; presets discord-free, email, whatsapp, smaller; under 10 minutes) on every PR; full matrix nightly on Linux CI and weekly on the lab VMs; hardware-encoder rows (`--encoders nvenc,amf,qsv,mf,videotoolbox,vaapi`) on the lab Mac, the Windows lab VM (Media Foundation software path), and Hunter's Windows PC through `winbridge` for NVENC on a real GPU.

## 7.4 Web tests (Playwright, real WASM)

`e2e/web/` runs against the production build: `npm run build -w apps/web`, then `wrangler pages dev apps/web/dist --port 5182`, which applies `_headers` the same way production does, so cross-origin isolation is tested for real. Projects: chromium, firefox, webkit (webkit on the macOS runner and the lab Mac).

Specs (each drops real fixture files through `setInputFiles` or a synthetic `DataTransfer` for folders, picks a destination, compresses, captures the download):

- `images.spec.ts`: photo, screenshot, transparent PNG, 23-file folder for Email; downloaded sizes under the limit; downloaded files re-verified in Node by calling the native `cia verify --preset <id> <file>` binary built in the same CI job.
- `pdf-office.spec.ts`, `archives.spec.ts`, `audio.spec.ts`.
- `video.spec.ts`: 2 min 1080p60 to Discord Free; portrait clip stays portrait; HDR clip (see below); unsupported container shows the right message.
- `refusals.spec.ts`: too long for WhatsApp shows the trim suggestion; no download is offered.
- `allowance.spec.ts`: the fourth file in a day is blocked; refusals don't consume allowance.
- `license.spec.ts`: activation against `e2e/mock-api/` (a small Node server implementing `/api/v1/license/*` and `/claims` with a test signing key that only `e2e` builds trust).
- `tour.spec.ts`, `@visual` screenshots of every main state per platform key (ConvertSave's approach, `E2E_SNAPSHOT_KEY`).
- `isolation.spec.ts`: `crossOriginIsolated === true`, no request leaves the origin except to the API host, the service worker serves the app offline.

HDR verification: the iPhone HDR fixture is compressed in each browser; the test extracts frames at 25, 50 and 75 percent from the web output and from the desktop FFmpeg output (the reference) and requires SSIMULACRA2 of at least 60 against the reference. Browsers that pass are recorded in `packages/webvideo/src/hdr-verified.ts` by a script, not by hand.

WASM unit tests: `wasm-bindgen-test` for `cia-wasm` in headless Chrome and Firefox (`cargo xtask wasm-test`).

## 7.5 Desktop end-to-end (WebdriverIO + tauri-driver)

Copy ConvertSave's `e2e/native/` harness (branch `test/e2e-suite`): `@wdio/tauri-service` with the external provider (tauri-driver plus WebKitWebDriver / msedgedriver) on Linux and Windows, the embedded provider (`tauri-plugin-wdio-webdriver` behind a cargo feature `e2e`) on macOS. Files are injected with `__TAURI_INTERNALS__.invoke("plugin:event|emit", { event: "tauri://drag-drop", ... })`. Visual comparisons with pixelmatch.

The `e2e` feature also: trusts the mock API's test license key, points `CIA_API_BASE` at `e2e/mock-api`, and points the FFmpeg manifest at a local HTTP server serving a test manifest signed with a test key.

Specs: first run and tour; image to Discord, then Copy file (clipboard checked with `Get-Clipboard -Format FileDropList` on Windows, `osascript -e 'the clipboard as «class furl»'` on macOS, `xclip -selection clipboard -t text/uri-list -o` on Linux); output naming with a pre-existing `photo (Discord).jpg`; FFmpeg setup (download, hash mismatch, success); video to Discord with progress and cancel (partial file removed); refusal card; activate, device-limit message, deactivate; Settings persistence.

Carry over ConvertSave's lab lessons: Windows native runs go through an interactive scheduled task (SSH lands in session 0), account lockout stays disabled on the Windows VM, the Mac must not lock its screen, wait for the license overlay before screenshots, and run `cargo test` and the `--features e2e` build into separate target directories so one does not overwrite the other.

## 7.6 Lab VMs

Reuse `ConvertSave/TestLab` rather than copying it. `tools/lab/cia_lab.py {start|sync|deps|build|matrix|web|native|smoke|pull} {linux|windows|mac}` calls `TestLab/lab.py` for VM control and SSH, and `TestLab/scripts/sync-checkout.sh <machine> CompressItAll` to send a Git bundle of the current commit. If a TestLab script needs a new parameter for this repo, add it there in a backward-compatible way; do not fork the scripts. Results go to `TestLab/results/<stamp>-<machine>-smidge/` in the formats the dashboard at http://test-results.floo.network already reads (Playwright `results.json`, JUnit XML, screenshot folders).

Per-machine deps script installs: Rust stable plus the wasm target, Node 22, wasi-sdk 25 and clang (Linux and Mac), WebKitGTK 4.1 dev packages (Linux), tauri-driver and the WebDriver for each OS, and the pinned Smidge FFmpeg from the Libraries manifest (not the system FFmpeg).

## 7.7 CI matrix (`test.yml`)

| Job | Runners | What |
|---|---|---|
| `lint` | ubuntu | `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `eslint`, `tsc -b`, `cargo deny check` (licences, bans, advisories), stale checks for ts-rs types, `licenses.json`, `presets-sources.md`, Supabase schema dump |
| `rust` | ubuntu-22.04, windows-latest, macos-latest | `cargo test --workspace --locked`, with the pinned Smidge FFmpeg installed from the Libraries manifest |
| `wasm` | ubuntu | install wasi-sdk 25 and clang; `cargo xtask wasm`; `cargo xtask wasm-test` (Chrome, Firefox); fail if the release `.wasm` exceeds 12 MB uncompressed |
| `matrix-smoke` | ubuntu, windows | `cia matrix --smoke`; exit code 2 fails the run |
| `web-e2e` | ubuntu (chromium, firefox), macos (webkit) | Playwright against `wrangler pages dev` |
| `site` | ubuntu with a `postgres:16` service | apply migrations, Vitest for API routes and the webhook handler using recorded Paddle sandbox payloads (`apps/site/test/fixtures/paddle/*.json`) signed in-test, including duplicate delivery, out-of-order delivery, renewal, past_due, cancel, refund pending then approved, chargeback and reversal |
| `desktop-build` | 3 OS | `tauri build --debug --no-bundle` to catch build breaks |
| `native-e2e` | ubuntu (xvfb), windows | WebdriverIO smoke specs |

`cargo deny` licence allowlist: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0, Unicode-3.0, Unicode-DFS-2016, CC0-1.0, IJG, MPL-2.0 (allowed only for the crates named in `deny.toml`'s exceptions list: symphonia family), OFL-1.1 (fonts). Anything GPL, AGPL, LGPL, SSPL, or unknown fails. The npm equivalent is `license-checker-rseidelsohn --onlyAllow` with the same list, plus MPL-2.0 for `mediabunny` only.

`nightly-matrix.yml`: full matrix on ubuntu at 07:00 UTC; uploads `matrix.md` as an artifact and posts a summary to the job page.

Site end-to-end with real Paddle sandbox checkout (test card), run on demand against a Vercel preview: `e2e/site/purchase.spec.ts` buys Lifetime and Yearly in sandbox, waits for the webhook, checks the success page key, activates the mock device, then issues a sandbox refund through the API and checks revocation.

---

# 8. Milestones

Ordered so a working demo exists as early as possible. "Blocked" means the milestone can be built and tested locally but cannot be finished without something from Hunter; the implementer does everything else first.

| # | Milestone | Done when | Blocked on Hunter |
|---|---|---|---|
| M0 | Repo skeleton | Monorepo layout from 2.3, npm and Cargo workspaces, `deny.toml`, `test.yml` lint and rust jobs green, `packages/brand` and `packages/tokens`, DECISIONS.md. Private repo `Hunter-Boone/CompressItAll` created with `gh` and pushed. | no |
| M1 | Engine core, images, CLI | Two-day WASM link spike first (2.5 item 4), decision recorded. `cia-core`, `cia-image`, `cia-mozjpeg`, `cia-webp`, `cia-archive`, `cia-license` (key and token code only), `cia-cli` (`compress`, `inspect`, `verify`, `matrix`). Image and archive rows of the smoke matrix pass on 3 OS. Demo: `cia compress photo.png --preset discord-free`. | no |
| M2 | Web app, images and archives | `apps/web` with the full main flow for images, folders and zips, worker pool, OPFS, downloads, Spotlight tour, free allowance, Licenses page. Running on the VM dev server and registered on the Links board. Web Playwright suite for these paths green. **First clickable demo.** | no (Cloudflare deploy waits for M8 items) |
| M3 | Desktop app, same scope | `apps/desktop` with native engine, save-next-to-original, copy file, drag-out, Settings, logs. Native e2e smoke green on Linux and Windows lab VMs. | no |
| M4 | PDF, Office, audio without FFmpeg | `cia-pdf`, `cia-office`, `cia-audio` (FLAC, WAV, native Opus; web Opus via WebCodecs). Matrix rows for these pass on 3 OS and in the web suite. | no |
| M5 | FFmpeg and desktop video | Smidge-Libraries workflow producing a pinned, signed FFmpeg; `cia-ffmpeg`; video planner; FFmpeg setup flow; trim; HEIC/AVIF/AAC input via FFmpeg. Synthetic and real-world video matrix rows pass on Linux, Windows (Media Foundation), Mac (VideoToolbox), and NVENC on Hunter's PC via winbridge. | Creating the **public** Libraries repo needs the final product name (it is baked into download URLs). Until then, build in a private staging repo `CompressItAll-Libraries-staging` and point the app at it. |
| M6 | Web video | `packages/webvideo`, capability checks, HDR verified list, video web specs green in Chromium, Firefox and WebKit. | no |
| M7 | Licensing backend | `apps/site` API, migrations, OTP dashboard, Paddle webhook, claims, emails; desktop and web license UI. All tests green against the mock API, local Postgres and recorded payloads. Then end-to-end against Paddle sandbox on a Vercel preview. | Paddle sandbox account, API key, client token, webhook secret, sandbox price ids; Supabase dev project; Resend API key and sending domain. |
| M8 | Release pipeline | `release-desktop.yml`, `promote.yml`, updater route, `web.yml`, `site.yml`; a signed beta (`v0.9.0-beta.1`) installs and self-updates to `beta.2` on all three lab machines. | Domain; Apple Developer ID certificate and notarisation credentials; Azure Trusted Signing access; Cloudflare account; Vercel project; creation of the public Downloads repo (needs the name). Unsigned builds and the full pipeline against a private staging repo can be finished first. |
| M9 | Polish and lab pass | Full matrix green on all lab machines; accessibility pass (keyboard-only run, contrast check, 130 percent text size); copy review against 4.1 tone rules; Licenses page complete; performance budgets met (a 48 MP PNG photo to Discord Free in under 6 s on desktop and 20 s on the web, on the lab VMs). | no |
| M10 | After launch | Threaded WASM build; `opus-rs` fallback for older Safari; ConvertSave-Libraries consolidation if wanted. | no |
| M11 | Launch | Production Supabase, production Paddle (domain review needs the live site with pricing, terms, refund policy, privacy policy and the legal seller name), DNS, Cloudflare and Vercel production deploys, `v1.0.0` promoted to stable. | Everything in section 9; production deploy approval. |

Suggested sequencing for one implementer: M0 → M1 → M2 (demo to Hunter) → M3 → M4 → M5 → M6, with M7 code starting in parallel once M3 is done, then M8, M9, M11.

---

# 9. Open questions for Hunter

Only items that need Hunter's decision, money, accounts or credentials. Everything else in this document is decided.

1. **Name.** Approve "Smidge", or pick Tuck, Sendsize or Fitbox. A trademark search in the US (and any other market you care about) is yours to commission. The name fixes the public repo names, the bundle identifier (`app.smidge.desktop`) and download URLs.
2. **Domain.** Which domain to buy (for example `smidge.app` or `getsmidge.com`). It sets `www.`, `app.`, `mail.` and the support address.
3. **Seller entity.** Pixel & Bracket LLC (named in ConvertSave's NOTICE) or Underscore Software LLC? It goes on the Paddle account, the terms of service (Paddle's domain review checks the legal name) and the code-signing publisher name.
4. **Prices.** Lifetime and Yearly amounts. Suggestion: $29 Lifetime and $12 per year, so Lifetime pays for itself in under three years and both sit below ConvertSave's price points for a narrower tool.
5. **Paddle account.** A second product in the existing ConvertSave Paddle account, or a new account? Then sandbox and live API keys, client-side tokens, the webhook secret and the two price ids.
6. **Code signing.** Reuse ConvertSave's Apple Developer team and Azure Trusted Signing profile (only if the seller entity is the same), or set up new ones.
7. **Accounts and spend.** A new Supabase project (paid tier for production), a Cloudflare account for Pages and DNS, a Vercel project, and a Resend sending domain.
8. **AAC posture.** The FFmpeg build Smidge downloads includes FFmpeg's built-in AAC encoder, exactly as ConvertSave's does today, and H.264 comes only from the operating system or GPU encoders. Confirm you accept the same posture for Smidge. Without the AAC encoder, desktop videos for WhatsApp and iMessage cannot be made (they need H.264 with AAC in MP4).
9. **Support address.** Which mailbox support goes to, and whether the existing ConvertSave support portal should also serve Smidge.

---
# 10. Third-party inventory

Versions are the latest on crates.io or npm on 2026-10-02 unless the wasm spike pinned an older one (noted). "wasm" means builds for `wasm32-unknown-unknown`: "yes" was measured by the 2026-10-02 spike or is pure Rust with a documented wasm path; "spike" means the C part compiled and linking is proven in M1; "n/a" means not used on the web. Licence texts and copyright lines are generated into the Licenses page (4.11).

## 10.1 Compiled into the desktop app and/or the WASM engine (Rust)

| Component | Version | Licence | Commercial OK | wasm | Where used |
|---|---|---|---|---|---|
| tauri, tauri-build | 2.11+ | MIT OR Apache-2.0 | yes | n/a | desktop shell |
| tauri-plugin-updater, -dialog, -log, -opener | 2.x | MIT OR Apache-2.0 | yes | n/a | updates, pickers, logs, reveal/open |
| tauri-plugin-drag | 2.1.1 | Apache-2.0 OR MIT | yes | n/a | drag result out of the window |
| clipboard-rs | 0.3.5 | MIT | yes | n/a | copy files to clipboard |
| windows | 0.61+ | MIT OR Apache-2.0 | yes | n/a | MachineGuid, Windows APIs |
| reqwest (rustls) | 0.12 | MIT OR Apache-2.0 | yes | n/a | license API, FFmpeg download |
| tokio | 1.53 | MIT | yes | n/a | desktop async |
| rayon | 1.12 | MIT OR Apache-2.0 | yes | n/a (M10 via wasm-bindgen-rayon) | parallel encodes |
| image | 0.25.10 | MIT OR Apache-2.0 | yes | yes | decoding, pixel ops |
| zune-jpeg | 0.5 | MIT OR Apache-2.0 OR Zlib | yes | yes | JPEG decode (via image) |
| mozjpeg-sys (mozjpeg C) | 2.2.3 | IJG AND Zlib AND BSD-3-Clause | yes; IJG requires the notice "This software is based in part on the work of the Independent JPEG Group" in documentation | spike (C compiles) | JPEG encode via our `cia-mozjpeg` wrapper |
| jpeg-encoder | 0.7.1 | (MIT OR Apache-2.0) AND IJG | yes | yes | web fallback JPEG encoder only if the spike fails |
| libwebp-sys (libwebp C) | 0.14.4 | MIT; libwebp BSD-3-Clause + Google patent grant | yes | spike (C compiles) | WebP encode via `cia-webp` |
| image-webp | 0.2.4 | MIT OR Apache-2.0 | yes | yes | WebP decode |
| oxipng (libdeflate C) | 10.2.1 (spike: 9) | MIT; libdeflate MIT | yes | yes, `default-features = false, features = ["freestanding"]`, no `parallel` | PNG optimisation |
| zopfli | 0.8.3 | Apache-2.0 | yes | yes | small PNG / zip entries |
| ravif | 0.13.0 | BSD-3-Clause | yes | yes (`rav1e/wasm`, no `asm`) | AVIF encode |
| rav1e | 0.8.1 | BSD-2-Clause + AOMedia Patent License 1.0 | yes; reproduce the patent licence | yes | AV1 for AVIF |
| gif | 0.14.2 | MIT OR Apache-2.0 | yes | yes | GIF decode/encode |
| quantette | 0.6.0 | MIT OR Apache-2.0 | yes | yes (pure Rust; confirm in spike) | palette PNG (replaces GPL libimagequant) |
| fast_image_resize | 6.1.0 | MIT OR Apache-2.0 | yes | yes (SIMD128 path) | downscaling |
| ssimulacra2 | 0.5.1 | BSD-2-Clause | yes | yes (pure Rust; confirm in spike) | perceptual score |
| kamadak-exif | 0.6.1 | BSD-2-Clause | yes | yes | read EXIF |
| little_exif | 0.6.23 | MIT OR Apache-2.0 | yes | yes | write kept EXIF |
| jxl-oxide | 0.12 | MIT OR Apache-2.0 | yes | yes | JPEG XL input |
| lopdf | 0.45.0 (spike: 0.36) | MIT | yes | yes (`wasm_js`, no `rayon`) | PDF optimiser |
| quick-xml | 0.42 | MIT | yes | yes | Office verification |
| zip | 8.6.0 (spike: 4) | MIT | yes | yes (trim features) | zip read/write |
| flate2 + zlib-rs | 1.1 / 0.6.8 | MIT OR Apache-2.0; Zlib | yes | yes | Deflate |
| sevenz-rust2 (+ lzma-rust2) | 0.23.0 / 0.21.0 (spike: 0.19) | Apache-2.0 | yes | yes (`default_wasm`) | 7z read/write |
| zstd (zstd C) | 0.13 (measured) / 0.14 | BSD-3-Clause (zstd is BSD OR GPLv2; we use BSD) | yes | yes (built-in wasm shim) | tar.zst |
| liblzma (xz C) | 0.4.8 | MIT OR Apache-2.0; xz 0BSD | yes | yes (`wasm` feature) | tar.xz, xz input |
| tar | 0.4.46 | MIT OR Apache-2.0 | yes | yes | tar read/write |
| bzip2 (libbz2-rs-sys) | 0.5+ | MIT OR Apache-2.0; bzip2 licence (BSD-like) | yes | yes | .bz2 input only |
| symphonia | 0.6.1 (spike: 0.5) | MPL-2.0 | yes, used unmodified; notice and source link | yes | audio decode (no aac/alac/isomp4 features) |
| flacenc | 0.5.1 | Apache-2.0 | yes | yes | FLAC encode |
| hound | 3.5.1 | Apache-2.0 | yes | yes | WAV |
| ogg | 0.9.2 | BSD-3-Clause | yes | yes | Ogg Opus mux |
| opus (libopus C) | 0.4.0 | MIT OR Apache-2.0; libopus BSD-3-Clause + royalty-free patent grants | yes | no (cmake) | native Opus encode/decode |
| rubato | 5.0.1 | MIT OR Apache-2.0 | yes | yes | resampling |
| infer | 0.22 | MIT | yes | yes | type detection |
| ed25519-dalek | 2.x or 3.0 | BSD-3-Clause | yes | yes | license and manifest signatures |
| sha2, base64, serde, serde_json, thiserror, ulid | current | MIT OR Apache-2.0 | yes | yes | general |
| ts-rs (build time) | 12.0.1 | MIT | yes | n/a | TS type generation |
| wasm-bindgen, js-sys, web-sys | 0.2.129 | MIT OR Apache-2.0 | yes | yes | web bindings |
| getrandom (`wasm_js`) | 0.3/0.4 | MIT OR Apache-2.0 | yes | yes | randomness on web |

## 10.2 JavaScript in the apps and site

| Component | Version | Licence | Commercial OK | Where used |
|---|---|---|---|---|
| react, react-dom | 18.3 | MIT | yes | apps |
| tailwindcss | 3.4 | MIT | yes | apps, site |
| lucide-react | current | ISC | yes | icons |
| vite | current | MIT | yes | build |
| @tauri-apps/api, plugins | 2.x | Apache-2.0 OR MIT | yes | desktop frontend |
| mediabunny | 1.61.0 | MPL-2.0 | yes, unmodified; notice and source link | web video and audio demux/mux |
| idb-keyval | 6.3.0 | Apache-2.0 | yes | web settings and tokens |
| zod | current | MIT | yes | API schemas |
| @fontsource/inter | 5.x | OFL-1.1 (font) | yes | UI font |
| @fontsource-variable/bricolage-grotesque | 5.x | OFL-1.1 (font) | yes | display font |
| next | 15.x | MIT | yes | site |
| @supabase/supabase-js | 2.x | MIT | yes | site server only |
| resend, @react-email/components | current | MIT | yes | site emails |
| @paddle/paddle-node-sdk | 3.10.0 | Apache-2.0 | yes | site server API calls (not webhook verification) |
| @paddle/paddle-js | 1.6.5 | Apache-2.0 (wrapper; the script it loads is Paddle's hosted service under Paddle's terms) | yes | site checkout only |

Development-only (not shipped): Playwright (Apache-2.0), WebdriverIO and @wdio/tauri-service (MIT), tauri-driver (Apache-2.0 OR MIT), Vitest (MIT), wrangler (MIT OR Apache-2.0), cargo-about (MIT OR Apache-2.0), cargo-deny (MIT OR Apache-2.0), license-checker-rseidelsohn (BSD-3-Clause), insta (Apache-2.0), wasm-bindgen-test (MIT OR Apache-2.0), binaryen/wasm-opt (Apache-2.0), wasi-sdk sysroot headers (Apache-2.0 WITH LLVM-exception; headers only, used at build time).

## 10.3 Downloaded on demand with the user's consent (never bundled)

| Component | Licence | Commercial OK | Where used |
|---|---|---|---|
| FFmpeg (Smidge-Libraries build) | LGPL-3.0-or-later (`--enable-version3`, no GPL, no nonfree); source published with every release | yes, as a separate process | desktop video, HEIC/AVIF/AAC input, MP3/AAC output |
| libvpx | BSD-3-Clause + WebM patent grant | yes | VP9 inside FFmpeg |
| libaom, dav1d | BSD-2-Clause + AOMedia Patent License 1.0 | yes | AV1/AVIF decode inside FFmpeg |
| libopus, libvorbis | BSD-3-Clause | yes | inside FFmpeg |
| LAME | LGPL-2.0 | yes, inside the separate FFmpeg process | MP3 encode |
| libwebp | BSD-3-Clause | yes | inside FFmpeg |
| zimg | WTFPL | yes | HDR tone-mapping (zscale) |
| nv-codec-headers, AMF headers, libvpl, libva | MIT | yes | GPU encoders inside FFmpeg |

## 10.4 Rejected, with the reason

| Component | Licence or problem | Replacement |
|---|---|---|
| libimagequant / imagequant crate / pngquant | GPL-3.0-or-later (or a paid licence) | quantette |
| dssim | AGPL-3.0 | ssimulacra2 |
| gifsicle | GPL-2.0 | gif crate + quantette |
| x264, x265, fdk-aac | GPL / non-free | platform and GPU H.264 encoders via FFmpeg; user's own FFmpeg if they choose |
| @ffmpeg/core (ffmpeg.wasm default build) | GPL-2.0-or-later (contains x264, x265) | WebCodecs + mediabunny |
| mp3lame-encoder, lame-sys, shine-rs | LGPL, statically linked | MP3 only through FFmpeg on desktop |
| @mediabunny/mp3-encoder, @mediabunny/aac-encoder | wraps LAME / libavcodec (LGPL) inside WASM | none on web |
| rusty_mp3 | new crate, code provenance could not be traced | revisit after an audit |
| ropus | README defers to libopus licence while crates.io says BSD-3; unclear | opus-rs (BSD-3) as the M10 candidate |
| `mozjpeg` wrapper crate | does not build for wasm (libc file APIs) | our `cia-mozjpeg` |
| `webp` crate | does not build for wasm | our `cia-webp` over libwebp-sys |
| Ghostscript | AGPL | lopdf |
| libheif | LGPL, and HEVC decode patents | FFmpeg process (desktop), Safari decoder (web) |
| unRAR | non-OSI licence that forbids building a RAR compressor from it | RAR not supported |
| Symphonia `aac`, `alac`, `isomp4` features | AAC is in an active patent pool (same posture as ConvertSave) | FFmpeg / WebCodecs |
| brotli output | licence is fine (BSD-3/MIT), but no consumer tool opens `.br` files | not offered |

No AGPL or SSPL component is used anywhere. Every GPL and LGPL item above is either rejected or runs only inside the separately downloaded FFmpeg process.
