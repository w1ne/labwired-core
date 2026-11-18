#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"

evidence_dir="examples/esp8684-devkitm-1-h4x/evidence"
scripts/onboarding_smoke.sh \
  --target-id esp8684-devkitm-1-h4x \
  --crate firmware-esp8684-devkitm-1-h4x-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp8684-devkitm-1-h4x/io-smoke.yaml \
  --system configs/systems/esp8684-devkitm-1-h4x.yaml \
  --out-dir "$evidence_dir"
