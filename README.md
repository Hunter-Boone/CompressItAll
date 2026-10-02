# CompressItAll (product name: Smidge, pending approval)

Make any file small enough to send. One Rust engine, compiled natively for the desktop app (Tauri 2) and to WebAssembly for the web app. Nothing is ever uploaded.

Read `docs/DESIGN.md` first: it is the specification. `docs/DECISIONS.md` records where the implementation deviates and why. `docs/presets-sources.md` lists every size limit with its source.

## Layout

| Path | What |
|---|---|
| `crates/cia-core` | job model, presets, budgets, per-message allocation, naming, free allowance, all user-facing copy |
| `crates/cia-image`, `cia-mozjpeg`, `cia-webp` | image planner (quality search, downscale, verification) over mozjpeg, libwebp, oxipng, ravif |
| `crates/cia-pdf`, `cia-office`, `cia-audio`, `cia-archive` | PDF, Office, audio and archive planners |
| `crates/cia-video-plan`, `cia-ffmpeg` | video budget math (shared) and the native FFmpeg runner |
| `crates/cia-engine` | orchestrator: detect, inspect, plan, run, verify, package, job log |
| `crates/cia-cli` | `cia` command line: `compress`, `plan`, `inspect`, `verify`, `matrix` |
| `crates/cia-license` | product keys, signed licence tokens, device ids |
| `crates/cia-wasm` | wasm-bindgen exports for the web app |
| `apps/desktop` | Tauri 2 app · `apps/web` browser app · `apps/site` marketing site and licence API |
| `packages/ui` | all screens (shared by desktop and web) · `packages/engine-client` host interface + mock · `packages/api-types` wire formats |
| `packages/presets`, `tokens`, `brand`, `licenses-data` | data the apps embed |
| `e2e/`, `fixtures/`, `tools/` | test suites, fixtures, build and lab scripts |

## Developing

Work in a local clone (`/home/hunter/work/CompressItAll` on the dev VM), never in the NAS checkout.

```
cargo test --workspace                                   # engine (needs ffmpeg on PATH for the video tests)
python3 tools/fixtures/generate.py --quick               # synthetic fixtures into fixtures/synth
cargo build -p cia-cli --release
CIA_FFMPEG=/usr/bin/ffmpeg target/release/cia compress --preset discord-free photo.jpg
CIA_FFMPEG=/usr/bin/ffmpeg target/release/cia matrix --fixtures fixtures/synth --presets smoke --out target/matrix --smoke
cargo xtask wasm                                         # WASM engine into apps/web/public/engine (needs /opt/wasi-sdk)
npm install && npm run dev -w apps/web                   # web app on :5181 (add ?mock=1 for the fake engine)
npm run dev -w apps/desktop                              # desktop app
npm run dev -w apps/site                                 # site on :3031
python3 tools/lab/cia_lab.py sync linux && python3 tools/lab/cia_lab.py build linux   # lab VMs
```

## Policies

- Every third-party component must be commercially redistributable (`cargo deny check`, `tools/licenses/generate.sh --check`). No GPL, LGPL, AGPL or SSPL in the shipped binaries.
- FFmpeg is never bundled. The desktop app downloads an LGPL-only build (pinned, hash-checked, signed manifest) after the user agrees, and runs it as a separate process.
- The honesty rule: an output is reported as fitting only after its written bytes are read back under the limit and decoded with a second decoder. `cia matrix` exits 2 on any violation; releases are blocked on it.
- Releases: `release-desktop.yml` builds a draft in the Downloads repo after tests pass; `promote.yml` publishes to a channel with a rollout percentage.
