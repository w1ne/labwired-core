#!/usr/bin/env bash
# EK-RA6M5 CLI proof: boot the RA6M5 smoke, assert the SCI0/LED/switch
# stream, then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ek-ra6m5
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra6m5/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv8m.main-none-eabi/release/firmware-ek-ra6m5-smoke \
  --system configs/systems/ek-ra6m5.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr v4.0.0 hello_world (tests/fixtures/r7fa6m5bh-zephyr-hello.elf):
# PSCU/CPSCU attribution, FSP clock bring-up on ra6_sysc, SCI0 in FIFO mode,
# then the full instruction-support audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra6m5/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa6m5bh-zephyr-hello.elf \
  --system configs/systems/ek-ra6m5.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr samples/synchronization: k_busy_wait on the SysTick cycle
# counter and k_msleep woken by SysTick (tests/fixtures/r7fa6m5bh-zephyr-sync.elf).
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra6m5/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
# Stock Zephyr v4.1.0 samples/drivers/counter/alarm: four AGT0 compare-match
# alarms, each linked to NVIC line 84 through ICU IELSR84
# (tests/fixtures/r7fa6m5bh-zephyr-counter-alarm.elf), then its audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra6m5/zephyr-counter-alarm.yaml \
  --output-dir "$OUT/zephyr-counter-alarm" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa6m5bh-zephyr-counter-alarm.elf \
  --system configs/systems/ek-ra6m5.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-counter-alarm-unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr v4.0.0 tests/drivers/uart/uart_basic_api: the interrupt-driven
# SCI0 driver, RXI/TXI/TEI linked to NVIC lines 0-2 through ICU IELSRn, with
# the 16-stage FIFO (tests/fixtures/r7fa6m5bh-zephyr-uart-basic-api.elf),
# then its audit. The audit's 150,000 steps cover boot only; the full
# ~45M-step run above is what exercises the interrupt paths.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra6m5/zephyr-uart-basic-api.yaml \
  --output-dir "$OUT/zephyr-uart-basic-api" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa6m5bh-zephyr-uart-basic-api.elf \
  --system configs/systems/ek-ra6m5.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-uart-basic-api-unsupported-audit" \
  --fail-on-unsupported
