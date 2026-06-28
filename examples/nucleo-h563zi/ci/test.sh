#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/nucleo-h563zi
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/io-smoke.yaml --output-dir "$OUT/io-smoke" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/nucleo-h563zi/fullchip-smoke.yaml --output-dir "$OUT/fullchip-smoke" --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv7m-none-eabi/release/firmware-h563-io-demo \
  --system configs/systems/nucleo-h563zi-demo.yaml \
  --max-steps 20000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  python3 - "$OUT/unsupported-audit/metrics.json" "NUCLEO-H563ZI" >> "$GITHUB_STEP_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
m = json.loads(Path(sys.argv[1]).read_text())
print(f"### {sys.argv[2]} Instruction Support\n")
print(f"- Instructions executed: `{m['instructions_executed']}`")
print(f"- Unsupported observations: `{m['unsupported_total']}`")
print(f"- Instruction support coverage: `{m['instruction_support_percent']}%`")
PY
fi
