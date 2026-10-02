# CompressItAll (product name: Smidge)

Make any file small enough to send. One Rust engine, compiled natively for the desktop app (Tauri 2) and to WebAssembly for the web app. Nothing is ever uploaded.

Read `docs/DESIGN.md` first. It is the specification; `docs/DECISIONS.md` records deviations.

## Layout

- `crates/` — the engine (`cia-core`, `cia-image`, `cia-pdf`, …) and the `cia` CLI
- `apps/desktop` — Tauri 2 desktop app · `apps/web` — browser app · `apps/site` — marketing site and licence API
- `packages/` — shared UI, tokens, presets, brand, engine client, API types
- `e2e/` — Playwright (web) and WebdriverIO (native) suites · `fixtures/` — test inputs
- `tools/lab/` — wrappers for the ConvertSave TestLab VMs

## Developing

Work in a local clone (`/home/hunter/work/CompressItAll` on the dev VM), never in the NAS checkout.

```
cargo test --workspace            # engine
cargo xtask fixtures              # synthetic fixtures (needs ffmpeg on PATH for video)
cargo run -p cia-cli -- compress photo.jpg --preset discord-free
cargo xtask wasm                  # build the WASM engine (needs /opt/wasi-sdk)
npm install && npm run dev -w apps/web
```

Engine policy: every third-party component must be commercially redistributable (`cargo deny check`). FFmpeg is never bundled; the desktop app downloads an LGPL-only build with the user's consent.
