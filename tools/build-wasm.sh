#!/usr/bin/env bash
# Build the web engine (DESIGN.md 2.5): cargo -> wasm-bindgen -> wasm-opt into
# apps/web/public/engine, then write apps/web/src/engine-version.json.
# Run as `cargo xtask wasm` or directly. Fails if the optimised .wasm is over 12 MB.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=apps/web/public/engine
NAME=cia_wasm
TARGET=wasm32-unknown-unknown
LIMIT=$((12 * 1024 * 1024))
SHA=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)

for tool in wasm-bindgen wasm-opt; do
  command -v "$tool" >/dev/null || { echo "::error::$tool not found on PATH"; exit 1; }
done
want=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | grep version | cut -d'"' -f2)
have=$(wasm-bindgen --version | awk '{print $2}')
if [ "$want" != "$have" ]; then
  echo "::error::wasm-bindgen-cli $have does not match the wasm-bindgen crate $want in Cargo.lock (cargo install wasm-bindgen-cli --version $want)"
  exit 1
fi
[ -d /opt/wasi-sdk ] || echo "::warning::/opt/wasi-sdk missing; the C codecs will not compile (see .cargo/config.toml)"

echo "== cargo build (release, $TARGET)"
CIA_GIT_SHA="$SHA" cargo build -p cia-wasm --release --target "$TARGET" "$@"

echo "== wasm-bindgen"
rm -rf "$OUT"
mkdir -p "$OUT"
# Hide reference-types from wasm-bindgen so it uses its JS heap instead of an
# externref table, which old binaryen (Debian 108, Ubuntu 22.04's 99) breaks.
# See tools/wasm-strip-features.py and DECISIONS.md.
python3 tools/wasm-strip-features.py "target/$TARGET/release/$NAME.wasm" "target/$TARGET/release/$NAME.stripped.wasm" reference-types
wasm-bindgen "target/$TARGET/release/$NAME.stripped.wasm" --target web --out-dir "$OUT" --out-name "$NAME" --no-typescript
# The .d.ts is not committed; the worker declares the exports it uses.

echo "== wasm-opt"
wasm-opt -O3 \
  --enable-bulk-memory --enable-nontrapping-float-to-int --enable-simd \
  --enable-sign-ext --enable-mutable-globals \
  "$OUT/${NAME}_bg.wasm" -o "$OUT/${NAME}_bg.wasm.opt"
mv "$OUT/${NAME}_bg.wasm.opt" "$OUT/${NAME}_bg.wasm"

raw=$(stat -c %s "$OUT/${NAME}_bg.wasm")
gz=$(gzip -9 -c "$OUT/${NAME}_bg.wasm" | wc -c)
js=$(stat -c %s "$OUT/${NAME}.js")
awk -v f="$OUT/${NAME}_bg.wasm" -v raw="$raw" -v gz="$gz" -v js="$js" -v name="$NAME" \
  'BEGIN { printf "%s: %d bytes (%.2f MB), gzip %d bytes (%.2f MB); %s.js %d bytes\n", f, raw, raw/1048576, gz, gz/1048576, name, js }'

cat > apps/web/src/engine-version.json <<JSON
{
  "sha": "$SHA",
  "builtAt": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "wasmBytes": $raw,
  "wasmGzipBytes": $gz
}
JSON
echo "wrote apps/web/src/engine-version.json"

if [ "$raw" -gt "$LIMIT" ]; then
  echo "::error::$OUT/${NAME}_bg.wasm is $raw bytes, over the $LIMIT byte budget"
  exit 1
fi
