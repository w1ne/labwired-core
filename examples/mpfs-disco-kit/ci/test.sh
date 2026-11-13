#!/usr/bin/env bash
# PolarFire SoC Discovery Kit (MPFS095T, RV64 U54 hart) CLI proof: stock Zephyr
# v4.1.0 hello_world, synchronization and tests/kernel/common, then require
# each image's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/mpfs-disco-kit
cargo run -q -p labwired-cli -- test \
  --script examples/mpfs-disco-kit/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
# synchronization busy-waits 100 ms per round (60 M instructions at 600 MHz)
# and kernel/common boots after a 500 ms busy-wait, so both get a release CLI.
for t in sync kernel-common; do
  cargo run -q --release -p labwired-cli -- test \
    --script "examples/mpfs-disco-kit/zephyr-$t.yaml" \
    --output-dir "$OUT/zephyr-$t" \
    --no-uart-stdout
done
for f in hello sync kernel-common; do
  ./scripts/unsupported_instruction_audit.sh \
    --firmware "tests/fixtures/mpfs-zephyr-$f.elf" \
    --system configs/systems/mpfs-disco-kit.yaml \
    --max-steps 2000000 \
    --out-dir "$OUT/unsupported-audit-$f" \
    --fail-on-unsupported
done
