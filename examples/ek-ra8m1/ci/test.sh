#!/usr/bin/env bash
# EK-RA8M1 CLI proof: boot the RA8M1 smoke, assert the SCI9/LED/switch
# stream, then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ek-ra8m1
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra8m1-smoke \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr v3.7.0 hello_world (tests/fixtures/r7fa8m1ah-zephyr-hello.elf):
# Armv8.1-M DLS/LE + CSINC, FSP clock bring-up on ra8_sysc, SCI_B console,
# then the full instruction-support audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-hello.elf \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr samples/synchronization: k_busy_wait on the SysTick cycle
# counter and k_msleep woken by SysTick (tests/fixtures/r7fa8m1ah-zephyr-sync.elf).
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
