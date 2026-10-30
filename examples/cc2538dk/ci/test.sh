#!/usr/bin/env bash
# CC2538DK CLI proof: boot the UART+LED smoke, assert the UART0/LED stream,
# then require the firmware's instruction stream to be fully supported.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/cc2538dk
cargo run -q -p labwired-cli -- test \
  --script examples/cc2538dk/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-cc2538dk-smoke \
  --system configs/systems/cc2538dk.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
# Stock Contiki-NG v4.9 hello-world through the clean-room ROM, RF core and
# sleep timer (tests/fixtures/cc2538dk-contiki-hello.elf).
cargo run -q -p labwired-cli -- test \
  --script examples/cc2538dk/contiki-hello.yaml \
  --output-dir "$OUT/contiki-hello" \
  --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/cc2538dk-contiki-hello.elf \
  --system configs/systems/cc2538dk.yaml \
  --max-steps 10000000 \
  --out-dir "$OUT/contiki-unsupported-audit" \
  --fail-on-unsupported
# Flash controller through the clean-room ROM's PageErase/ProgramFlash:
# the steps of Renode's cc2538_flash_controller.robot, then stock
# Contiki-NG v4.9 CFS/Coffee (examples/storage/cfs-coffee).
for script in flash-controller contiki-coffee contiki-test-cfs; do
  cargo run -q -p labwired-cli -- test \
    --script "examples/cc2538dk/$script.yaml" \
    --output-dir "$OUT/$script" \
    --no-uart-stdout
done
./scripts/unsupported_instruction_audit.sh \
  --firmware tests/fixtures/cc2538dk-contiki-test-cfs.elf \
  --system configs/systems/cc2538dk.yaml \
  --max-steps 20000000 \
  --out-dir "$OUT/cfs-unsupported-audit" \
  --fail-on-unsupported
