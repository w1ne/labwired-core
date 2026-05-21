#!/usr/bin/env bash
#
# Fetch pre-stripped demo firmware ELFs from the public GitHub Release
# (w1ne/labwired-core/releases/tag/{release_tag}) so they don't bloat
# the monorepo's git history. Hosted on labwired-core (public) because
# this monorepo is private — the release URL has to be unauthenticated
# curl-able from Cloudflare Pages build CI.
#
# Idempotent — files that already exist locally with at least the
# expected `minSizeBytes` are kept as-is. Bump the release tag in
# demo-assets.json (firmware-demos-v2, v3, …) when publishing a new
# firmware revision so deploys are reproducible and old branches keep
# pulling the bytes they were built against.
#
# The list of assets to fetch lives in `packages/playground/demo-assets.json`
# — the same JSON is also imported by bundled-configs.ts so the playground
# UI and the build-time fetch stay aligned on a single source of truth.
#
# Wired into `npm run prebuild` + `npm run predev` so it fires before
# vite ever reads the public/ directory.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLAYGROUND_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WASM_DIR="$PLAYGROUND_ROOT/public/wasm"
MANIFEST="$PLAYGROUND_ROOT/demo-assets.json"
mkdir -p "$WASM_DIR"

if [[ ! -f "$MANIFEST" ]]; then
  echo "[fetch-demo-firmware] ERROR: $MANIFEST not found" >&2
  exit 1
fi

# Parse the manifest with Node (always available — vite needs it). Emits
# one tab-separated line per asset: filename<TAB>url<TAB>minSizeBytes.
mapfile -t ROWS < <(node -e '
  const m = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
  const tag = m.release_tag;
  const base = m.release_base_url;
  for (const a of m.assets) {
    process.stdout.write(`${a.filename}\t${base}/${tag}/${a.filename}\t${a.minSizeBytes}\n`);
  }
' "$MANIFEST")

if [[ ${#ROWS[@]} -eq 0 ]]; then
  echo "[fetch-demo-firmware] manifest has zero assets, nothing to do"
  exit 0
fi

for row in "${ROWS[@]}"; do
  IFS=$'\t' read -r name url min_size <<<"$row"
  out="$WASM_DIR/$name"

  if [[ -f "$out" ]]; then
    actual_size=$(wc -c <"$out")
    if (( actual_size >= min_size )); then
      echo "[fetch-demo-firmware] $name already present (${actual_size} bytes), skip"
      continue
    fi
    echo "[fetch-demo-firmware] $name too small (${actual_size} < ${min_size}), refetching"
    rm -f "$out"
  fi

  echo "[fetch-demo-firmware] fetching $name from $url"
  curl -fsSL --retry 3 --retry-delay 2 "$url" -o "$out.tmp"
  actual_size=$(wc -c <"$out.tmp")
  if (( actual_size < min_size )); then
    echo "[fetch-demo-firmware] ERROR: $name fetched only ${actual_size} bytes (< ${min_size}). Did the release tag move?" >&2
    rm -f "$out.tmp"
    exit 1
  fi
  mv "$out.tmp" "$out"
  echo "[fetch-demo-firmware] $name → ${actual_size} bytes"
done
