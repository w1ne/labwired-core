#!/usr/bin/env bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/pic18f4520
FW=examples/pic18f4520/firmware
mkdir -p "$OUT"

# The committed ELF must be exactly the committed SDCC HEX, wrapped.
python3 "$FW/hex2elf.py" "$FW/pic18f4520-sdcc-uart-smoke.hex" "$OUT/rewrapped.elf"
cmp "$OUT/rewrapped.elf" "$FW/pic18f4520-sdcc-uart-smoke.elf"

cargo run -q -p labwired-cli -- test \
  --script examples/pic18f4520/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout

# A PIC18 decode error aborts the run, so a clean 200k-step exit is the
# unsupported-instruction signal for this core (the audit's per-pattern
# counters are ARM/RISC-V only).
./scripts/unsupported_instruction_audit.sh \
  --firmware "$FW/pic18f4520-sdcc-uart-smoke.elf" \
  --system configs/systems/pic18f4520-demo.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
grep -q '"sim_exit_code": 0' "$OUT/unsupported-audit/metrics.json"
