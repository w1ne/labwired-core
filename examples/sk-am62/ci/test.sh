#!/usr/bin/env bash
# SK-AM62 (TI AM6234, Cortex-A53 AArch64 core) CLI proof: stock Zephyr v4.3.0
# hello_world, synchronization and tests/kernel/common, then require each
# image's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/sk-am62
# Literal --script paths: scripts/ci/chip_coverage.py only credits a chip for
# a script path it can read verbatim, not one assembled in a loop.
cargo run -q -p labwired-cli -- test \
  --script examples/sk-am62/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
# synchronization busy-waits 100 ms per round (140 M instructions at 1.4 GHz)
# and kernel/common boots after a 500 ms busy-wait, so both get a release CLI.
cargo run -q --release -p labwired-cli -- test \
  --script examples/sk-am62/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
cargo run -q --release -p labwired-cli -- test \
  --script examples/sk-am62/zephyr-kernel-common.yaml \
  --output-dir "$OUT/zephyr-kernel-common" \
  --no-uart-stdout
for f in hello sync kernel-common; do
  ./scripts/unsupported_instruction_audit.sh \
    --firmware "tests/fixtures/sk-am62-zephyr-$f.elf" \
    --system configs/systems/sk-am62.yaml \
    --max-steps 2000000 \
    --out-dir "$OUT/unsupported-audit-$f" \
    --fail-on-unsupported
done
