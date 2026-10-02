# Implementation decisions

One line per non-obvious decision: what was chosen, why, what was tested. Design-level decisions live in DESIGN.md; this file records the implementer's deviations and findings.

- 2026-10-02 · Product name "Smidge" is the architect's proposal, not yet approved by Hunter. Code names use `cia`, the brand lives only in `packages/brand/brand.json`, so a rename touches one file plus repo names.
- 2026-10-02 · wasi-sdk 25 is expected at `/opt/wasi-sdk` (symlink on the dev VM). The spike that proved mozjpeg/libwebp/libdeflate/zstd compile for wasm32 with that sysroot plus `-D__wasi__` is recorded in `.cargo/config.toml`.
- 2026-10-02 · Toolchain pinned to 1.98.1 (the version on the dev VM and both lab VMs). Hunter's Windows PC has 1.92; rustup installs the pin on first build there.
