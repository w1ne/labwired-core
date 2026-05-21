#!/usr/bin/env bash
#
# Fetch pre-stripped demo firmware ELFs from the public GitHub Release
# (w1ne/labwired-core/releases/tag/firmware-demos-v1) so they don't bloat
# the git history of this monorepo. Hosted on labwired-core (public)
# because the labwired monorepo itself is private — the release URL has
# to be unauthenticated curl-able from Cloudflare Pages build CI.
#
# Idempotent — files that already exist locally are kept as-is. Bump
# the release tag (firmware-demos-v2, v3, …) when you publish a new
# firmware revision so deploys are reproducible and old branches keep
# pulling the bytes they were built against.
#
# Wired into `npm run prebuild` + `npm run predev` so it fires before
# `vite build` / `vite` ever read the public/ directory.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLAYGROUND_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WASM_DIR="$PLAYGROUND_ROOT/public/wasm"
mkdir -p "$WASM_DIR"

RELEASE_TAG="firmware-demos-v1"
BASE_URL="https://github.com/w1ne/labwired-core/releases/download/$RELEASE_TAG"

# (filename, expected_minimum_size_bytes) — size check catches a corrupted
# fetch (e.g., a GitHub 404 page being saved as the .elf).
FILES=(
  "demo-agentdeck.elf:1000000"
)

for entry in "${FILES[@]}"; do
  name="${entry%%:*}"
  min_size="${entry##*:}"
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

  echo "[fetch-demo-firmware] fetching $name from $RELEASE_TAG"
  curl -fsSL --retry 3 --retry-delay 2 \
    "$BASE_URL/$name" -o "$out.tmp"
  actual_size=$(wc -c <"$out.tmp")
  if (( actual_size < min_size )); then
    echo "[fetch-demo-firmware] ERROR: $name fetched only ${actual_size} bytes (< ${min_size}). Did the release tag move?" >&2
    rm -f "$out.tmp"
    exit 1
  fi
  mv "$out.tmp" "$out"
  echo "[fetch-demo-firmware] $name → ${actual_size} bytes"
done
