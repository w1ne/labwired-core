#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"

evidence_dir="examples/esp32c3-devkitm-1-n4x/evidence"
scripts/onboarding_smoke.sh \
  --target-id esp32c3-devkitm-1-n4x \
  --crate firmware-esp32c3-devkitm-1-n4x-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c3-devkitm-1-n4x/io-smoke.yaml \
  --system configs/systems/esp32c3-devkitm-1-n4x.yaml \
  --out-dir "$evidence_dir"

python3 examples/esp32c3-devkitm-1-n4x/ci/verify-rgb-evidence.py \
  "$evidence_dir/simulation-output/result.json" \
  2>&1 | tee "$evidence_dir/logs/verify_rgb.log"
