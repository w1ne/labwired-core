#!/usr/bin/env bash
# Curiosity PIC32MZ EF 2.0 (PIC32MZ2048EFM144, MIPS32r2 M5150 core) CLI
# proof: stock upstream NuttX nsh with a typed `hello`, then require both
# committed images' instruction streams to be fully supported. The full
# ostest (examples/curiosity-pic32mz-ef/nuttx-ostest.yaml) is a manual run.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/curiosity-pic32mz-ef
# Literal --script paths: scripts/ci/chip_coverage.py only credits a chip for
# a script path it can read verbatim, not one assembled in a loop.
cargo run -q -p labwired-cli -- test \
  --script examples/curiosity-pic32mz-ef/nuttx-nsh.yaml \
  --output-dir "$OUT/nuttx-nsh" \
  --no-uart-stdout
for f in nsh ostest; do
  ./scripts/unsupported_instruction_audit.sh \
    --firmware "tests/fixtures/curiosity-pic32mz-ef-nuttx-$f.elf" \
    --system configs/systems/curiosity-pic32mz-ef.yaml \
    --max-steps 2000000 \
    --out-dir "$OUT/unsupported-audit-$f" \
    --fail-on-unsupported
done
