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
# Stock Zephyr v4.0.0 samples/drivers/counter/alarm: four AGT0 compare-match
# alarms, each linked to NVIC line 84 through ICU IELSR84
# (tests/fixtures/r7fa8m1ah-zephyr-counter-alarm.elf), then its audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-counter-alarm.yaml \
  --output-dir "$OUT/zephyr-counter-alarm" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-counter-alarm.elf \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-counter-alarm-unsupported-audit" \
  --fail-on-unsupported
# Stock Zephyr v3.7.0 tests/drivers/uart/uart_basic_api: the interrupt-driven
# SCI_B driver, RXI/TXI/TEI linked through ICU IELSR24-26
# (tests/fixtures/r7fa8m1ah-zephyr-uart-basic-api.elf), then its audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-uart-basic-api.yaml \
  --output-dir "$OUT/zephyr-uart-basic-api" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-uart-basic-api.elf \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 150000 \
  --out-dir "$OUT/zephyr-uart-basic-api-unsupported-audit" \
  --fail-on-unsupported

# Stock Zephyr v4.1.0 samples/basic/button: two S1 presses (P009, IRQ13 on the
# falling edge, linked to NVIC line 89 through ICU IELSR89) print exactly two
# "Button pressed" lines (tests/fixtures/r7fa8m1ah-zephyr-button.elf), then its
# audit, run past both presses.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-button.yaml \
  --output-dir "$OUT/zephyr-button" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-button.elf \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 2000000 \
  --out-dir "$OUT/zephyr-button-unsupported-audit" \
  --fail-on-unsupported

# Stock Zephyr v4.1.0 samples/basic/blinky: LED1 (P600) made an output through
# PmnPFS.PDR|PODR and toggled five times through the 16-bit PCNTR1.PODR alias
# (tests/fixtures/r7fa8m1ah-zephyr-blinky.elf), then its audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-blinky.yaml \
  --output-dir "$OUT/zephyr-blinky" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-blinky.elf \
  --system configs/systems/ek-ra8m1.yaml \
  --max-steps 1000000 \
  --out-dir "$OUT/zephyr-blinky-unsupported-audit" \
  --fail-on-unsupported

# Stock Zephyr v4.1.0 tests/drivers/adc/adc_api: ADC1 channels 0 and 2 (a
# potentiometer and a TMP36 in adc-system.yaml), every scan end linked to NVIC
# line 39 through ICU IELSR39 (tests/fixtures/r7fa8m1ah-zephyr-adc-api.elf),
# 6/6, then its audit.
cargo run -q -p labwired-cli -- test \
  --script examples/ek-ra8m1/zephyr-adc-api.yaml \
  --output-dir "$OUT/zephyr-adc-api" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/r7fa8m1ah-zephyr-adc-api.elf \
  --system examples/ek-ra8m1/adc-system.yaml \
  --max-steps 2000000 \
  --out-dir "$OUT/zephyr-adc-api-unsupported-audit" \
  --fail-on-unsupported
