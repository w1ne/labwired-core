#!/usr/bin/env bash
# MIMXRT595-EVK CLI proof: boot the MIMXRT595S smoke, assert the FLEXCOMM0/
# GPIO LED/switch stream, then require the firmware's instruction stream to be
# fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/mimxrt595-evk
cargo run -q -p labwired-cli -- test \
  --script examples/mimxrt595-evk/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-mimxrt595-evk-smoke \
  --system configs/systems/mimxrt595-evk.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr v3.7.0 hello_world (tests/fixtures/mimxrt595-zephyr-hello.elf):
# the secure-alias map, CLKCTL hand-shakes, boot-ROM OTP API, FlexSPI0
# re-clock and the OS timer, then the full instruction-support audit.
cargo run -q -p labwired-cli -- test \
  --script examples/mimxrt595-evk/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/mimxrt595-zephyr-hello.elf \
  --system configs/systems/mimxrt595-evk.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr samples/synchronization: k_busy_wait on the OS timer count and
# k_msleep woken by the OS timer match IRQ (tests/fixtures/mimxrt595-zephyr-sync.elf).
cargo run -q -p labwired-cli -- test \
  --script examples/mimxrt595-evk/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
