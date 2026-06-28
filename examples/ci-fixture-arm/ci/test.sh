#!/usr/bin/env bash
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/ci-fixture-arm
cargo run -q -p labwired-cli -- test --script examples/ci/uart-ok.yaml --output-dir "$OUT/smoke" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/ci/dummy-max-uart-bytes.yaml --output-dir "$OUT/max-uart" --no-uart-stdout
cargo run -q -p labwired-cli -- test --script examples/ci/dummy-no-progress.yaml --output-dir "$OUT/no-progress" --no-uart-stdout
./scripts/unsupported_instruction_audit.sh \
  --firmware target/thumbv6m-none-eabi/release/firmware-ci-fixture \
  --system configs/systems/ci-fixture-uart1.yaml \
  --max-steps 5000 \
  --out-dir "$OUT/unsupported-audit" \
  --fail-on-unsupported
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  python3 - "$OUT/unsupported-audit/metrics.json" "CI Fixture ARM" >> "$GITHUB_STEP_SUMMARY" <<'PY'
import json, sys
from pathlib import Path
m = json.loads(Path(sys.argv[1]).read_text())
print(f"### {sys.argv[2]} Instruction Support\n")
print(f"- Instructions executed: `{m['instructions_executed']}`")
print(f"- Unsupported observations: `{m['unsupported_total']}`")
print(f"- Instruction support coverage: `{m['instruction_support_percent']}%`")
PY
fi
