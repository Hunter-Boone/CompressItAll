# Implementation decisions

One line per non-obvious decision: what was chosen, why, what was tested. Design-level decisions live in DESIGN.md; this file records the implementer's deviations and findings.

- 2026-10-02 · Product name "Smidge" is the architect's proposal, not yet approved by Hunter. Code names use `cia`, the brand lives only in `packages/brand/brand.json`, so a rename touches one file plus repo names.
- 2026-10-02 · wasi-sdk 25 is expected at `/opt/wasi-sdk` (symlink on the dev VM). The spike that proved mozjpeg/libwebp/libdeflate/zstd compile for wasm32 with that sysroot plus `-D__wasi__` is recorded in `.cargo/config.toml`.
- 2026-10-02 · Toolchain pinned to 1.98.1 (the version on the dev VM and both lab VMs). Hunter's Windows PC has 1.92; rustup installs the pin on first build there.
- 2026-10-02 · Product key check character is Luhn mod 31 with the weighted sum reduced mod 31 instead of the textbook `a / N + a % N` fold. The fold is only a bijection for even N; with N = 31 it maps both 1 (B) and 16 (T) to 2, so a B/T typo in a doubled position would pass. Reducing mod the prime keeps weights 1 and 2 invertible: the exhaustive test (200 keys × 19 positions × 30 alternatives) and the adjacent-transposition test both pass, in Rust and TypeScript.
- 2026-10-02 · `cia-license` key `2026-10` is a PLACEHOLDER pair whose seed sits in `crates/cia-license/test-keys.json`. Before launch: `cargo xtask keygen`, paste the public key into `keys.rs`, set `LICENSE_SIGNING_KEY` on Vercel, delete the placeholder entry. The `test` kid is compiled in only under `cfg(test)` or feature `e2e`.
- 2026-10-02 · `device::device_hash()` falls back to hashing the hostname when no machine id can be read (no `/etc/machine-id`, broken registry) so activation never hard-fails; the raw id is still never sent. `machine_id()` is exposed separately and returns an error in that case.
- 2026-10-02 · `normalise` strips an explicit set (ASCII whitespace, NBSP, dash) rather than Rust `is_whitespace` / JS `\s`, which disagree on U+0085 and U+FEFF; both implementations share the list.
- 2026-10-02 · `@cia/api-types` verifies tokens with WebCrypto Ed25519 and falls back to `@noble/ed25519` (MIT) when `importKey("Ed25519")` rejects (Safari < 17). `signToken` (noble, server only) is byte-identical to Rust `token::sign`; the Vitest suite proves it by re-signing the shared vectors.
