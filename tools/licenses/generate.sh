#!/usr/bin/env bash
# Regenerates packages/licenses-data/licenses.json and NOTICE.md (DESIGN.md 4.11).
#   tools/licenses/generate.sh          write the files
#   tools/licenses/generate.sh --check  regenerate into a temp dir and diff; exit 1 when stale
# Run through `cargo xtask licenses`. Needs cargo-about (`cargo install cargo-about --features cli`),
# node and an installed npm workspace.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HERE="$ROOT/tools/licenses"
DATA_DIR="$ROOT/packages/licenses-data"
OUT_JSON="$DATA_DIR/licenses.json"
OUT_NOTICE="$ROOT/NOTICE.md"

CHECK=0
for a in "$@"; do
  case "$a" in
    --check) CHECK=1 ;;
    *) echo "usage: $0 [--check]" >&2; exit 2 ;;
  esac
done

command -v cargo-about >/dev/null || { echo "cargo-about is not installed: cargo install cargo-about --locked --features cli" >&2; exit 2; }
command -v node >/dev/null || { echo "node is required" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Shipped Rust artefacts: package -> target triples it is built for.
# cia-wasm (the web engine) joins when the crate exists.
declare -a UNITS=("cia-cli:x86_64-unknown-linux-gnu,x86_64-pc-windows-msvc,aarch64-apple-darwin,x86_64-apple-darwin")
if [ -f "$ROOT/crates/cia-wasm/Cargo.toml" ]; then
  UNITS+=("cia-wasm:wasm32-unknown-unknown")
fi

# 1. cargo-about once per shipped package (the wasm engine pulls crates the CLI never sees), then
#    union the results into one file.
cd "$ROOT"
ABOUTS=()
for unit in "${UNITS[@]}"; do
  pkg="${unit%%:*}"
  cargo about generate -c about.toml --locked -m "crates/$pkg/Cargo.toml" -o "$WORK/about-$pkg.json" tools/licenses/about.hbs
  ABOUTS+=("$WORK/about-$pkg.json")
done
python3 - "$WORK/rust-about.json" "${ABOUTS[@]}" <<'PY'
import json, sys
out, *ins = sys.argv[1:]
by_id = {}
for f in ins:
    for lic in json.load(open(f))["licenses"]:
        cur = by_id.setdefault(lic["id"], {**lic, "used_by": []})
        seen = {(u.get("crate", u).get("name"), u.get("crate", u).get("version")) for u in cur["used_by"]}
        for u in lic.get("used_by", []):
            key = (u.get("crate", u).get("name"), u.get("crate", u).get("version"))
            if key not in seen:
                cur["used_by"].append(u); seen.add(key)
json.dump({"licenses": sorted(by_id.values(), key=lambda l: l["id"])}, open(out, "w"), indent=1)
PY

# 2. Exact dependency set of each shipped package, normal deps only, per target.
TREES=()
for unit in "${UNITS[@]}"; do
  pkg="${unit%%:*}"
  IFS=',' read -r -a triples <<<"${unit#*:}"
  for t in "${triples[@]}"; do
    f="$WORK/tree-$pkg-$t.txt"
    cargo tree -p "$pkg" -e normal --target "$t" --prefix none --format "{p}" --locked >"$f"
    TREES+=("$f")
  done
done
TREE_LIST="$(IFS=,; echo "${TREES[*]}")"

# 3. npm: every production dependency reachable from apps/web (node_modules is hoisted at the root,
#    so the licence scan runs over the whole tree and merge.mjs keeps only what npm ls lists).
npm ls --omit=dev --all --json -w apps/web >"$WORK/npm-tree.json" 2>/dev/null || true
npx --no-install license-checker-rseidelsohn --json --excludePrivatePackages >"$WORK/npm-licenses.json"

# 4. Merge.
if [ "$CHECK" = 1 ]; then
  [ -f "$OUT_JSON" ] || { echo "licenses: $OUT_JSON does not exist; run cargo xtask licenses" >&2; exit 1; }
  GENERATED_AT="$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")).generated_at)' "$OUT_JSON")"
  TARGET_JSON="$WORK/licenses.json"
  TARGET_NOTICE="$WORK/NOTICE.md"
else
  GENERATED_AT="${LICENSES_GENERATED_AT:-$(date -u +%Y-%m-%d)}"
  mkdir -p "$DATA_DIR"
  TARGET_JSON="$OUT_JSON"
  TARGET_NOTICE="$OUT_NOTICE"
fi

node "$HERE/merge.mjs" \
  --root "$ROOT" \
  --texts "$HERE/texts" \
  --extra "$HERE/extra.json" \
  --generated-at "$GENERATED_AT" \
  --rust "$WORK/rust-about.json" \
  --rust-trees "$TREE_LIST" \
  --npm-tree "$WORK/npm-tree.json" \
  --npm-licenses "$WORK/npm-licenses.json" \
  --out-json "$TARGET_JSON" \
  --out-notice "$TARGET_NOTICE"

if [ "$CHECK" = 1 ]; then
  stale=0
  diff -u "$OUT_JSON" "$TARGET_JSON" || stale=1
  diff -u "$OUT_NOTICE" "$TARGET_NOTICE" || stale=1
  if [ "$stale" = 1 ]; then
    echo "licenses: packages/licenses-data/licenses.json or NOTICE.md is stale; run cargo xtask licenses" >&2
    exit 1
  fi
  echo "licenses: up to date"
else
  echo "wrote $OUT_JSON and $OUT_NOTICE"
fi
