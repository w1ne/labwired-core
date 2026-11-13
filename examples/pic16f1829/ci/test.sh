#!/usr/bin/env bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/pic16f1829
FW=examples/pic16f1829/firmware
mkdir -p "$OUT"

# The committed ELF must be exactly the committed SDCC HEX, wrapped.
python3 "$FW/hex2elf.py" "$FW/pic16f1829-sdcc-uart-smoke.hex" "$OUT/rewrapped.elf"
cmp "$OUT/rewrapped.elf" "$FW/pic16f1829-sdcc-uart-smoke.elf"

cargo run -q -p labwired-cli -- test \
  --script examples/pic16f1829/io-smoke.yaml \
  --output-dir "$OUT/io-smoke" \
  --no-uart-stdout

# A PIC16 decode error ends the run with exit 3 (EXIT_RUNTIME_ERROR), which
# the audit records as sim_exit_code, so a clean 200k-step exit is the
# unsupported-instruction signal for this core (the audit's per-pattern
# counters are ARM/RISC-V only).
./scripts/unsupported_instruction_audit.sh \
  --firmware "$FW/pic16f1829-sdcc-uart-smoke.elf" \
  --system configs/systems/pic16f1829-curiosity.yaml \
  --max-steps 200000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
grep -q '"sim_exit_code": 0' "$OUT/unsupported-audit/metrics.json"
