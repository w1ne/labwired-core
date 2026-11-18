#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo_root"

evidence_dir="examples/esp32h2-devkitm-1-n4s/evidence"
scripts/onboarding_smoke.sh \
  --target-id esp32h2-devkitm-1-n4s \
  --crate firmware-esp32h2-demo \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32h2-devkitm-1-n4s/io-smoke.yaml \
  --system configs/systems/esp32h2-devkitm-1-n4s.yaml \
  --out-dir "$evidence_dir"

python3 examples/esp32h2-devkitm-1-n4s/ci/verify-rgb-evidence.py \
  "$evidence_dir/simulation-output/result.json" \
  2>&1 | tee "$evidence_dir/logs/verify_rgb.log"
