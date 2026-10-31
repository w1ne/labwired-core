#!/usr/bin/env bash
# NUCLEO-L552ZE-Q CLI proof: boot the L552 smoke, assert the LPUART1/LED/button
# stream, boot the stock Zephyr hello_world and synchronization samples, then
# require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-l552ze-q
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-l552ze-q/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
# Stock Zephyr v3.7.0 hello_world + synchronization (committed fixtures, no
# Zephyr build needed).
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-l552ze-q/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-l552ze-q/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
# Zephyr tests/kernel/fpu_sharing/generic runs 899 M cycles (~75 s on a
# release build, far longer on debug), so it gets a release CLI.
cargo run -q --release -p labwired-cli -- test \
  --script examples/nucleo-l552ze-q/zephyr-fpu-sharing.yaml \
  --output-dir "$OUT/zephyr-fpu-sharing" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-l552ze-smoke \
  --system configs/systems/nucleo-l552ze-q.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
