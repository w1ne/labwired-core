#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${REPO_ROOT}"

EVIDENCE_DIR="${REPO_ROOT}/examples/esp32c5-devkitc-1-bme280-smoke/evidence"
mkdir -p "${EVIDENCE_DIR}/logs"

"${REPO_ROOT}/examples/esp32c5-devkitc-1-bme280-smoke/ci/build.sh" \
  2>&1 | tee "${EVIDENCE_DIR}/logs/build_firmware.log"
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc-1-bme280-smoke/io-smoke.yaml \
  --output-dir "${EVIDENCE_DIR}/simulation-output" --no-uart-stdout \
  2>&1 | tee "${EVIDENCE_DIR}/logs/run_smoke.log"
python3 examples/esp32c5-devkitc-1-bme280-smoke/ci/verify-rgb-evidence.py \
  "${EVIDENCE_DIR}/simulation-output/result.json" \
  2>&1 | tee "${EVIDENCE_DIR}/logs/verify_rgb.log"
