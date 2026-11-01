#!/usr/bin/env bash
# EK-RX261 (R5F52618BGFP, RXv3 core) CLI proof: stock Zephyr v4.3.0
# hello_world, synchronization and tests/kernel/common, then require each
# image's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ek-rx261
# Literal --script paths: scripts/ci/chip_coverage.py only credits a chip for
# a script path it can read verbatim, not one assembled in a loop.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-rx261/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/ek-rx261/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/ek-rx261/zephyr-kernel-common.yaml \
  --output-dir "$OUT/zephyr-kernel-common" \
  --no-uart-stdout
for f in hello sync kernel-common; do
  ./scripts/unsupported_instruction_audit.sh \
    --firmware "tests/fixtures/ek-rx261-zephyr-$f.elf" \
    --system configs/systems/ek-rx261.yaml \
    --max-steps 2000000 \
    --out-dir "$OUT/unsupported-audit-$f" \
    --fail-on-unsupported
done
