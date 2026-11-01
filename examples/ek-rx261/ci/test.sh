#!/usr/bin/env bash
# EK-RX261 (R5F52618BGFP, RXv3 core) CLI proof: stock Zephyr v4.3.0
# hello_world, synchronization and tests/kernel/common, then require each
# image's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ek-rx261
for t in hello sync kernel-common; do
  cargo run -q -p labwired-cli -- test \
    --script "examples/ek-rx261/zephyr-$t.yaml" \
    --output-dir "$OUT/zephyr-$t" \
    --no-uart-stdout
done
for f in hello sync kernel-common; do
  ./scripts/unsupported_instruction_audit.sh \
    --firmware "tests/fixtures/ek-rx261-zephyr-$f.elf" \
    --system configs/systems/ek-rx261.yaml \
    --max-steps 2000000 \
    --out-dir "$OUT/unsupported-audit-$f" \
    --fail-on-unsupported
done
