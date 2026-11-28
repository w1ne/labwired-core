#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."

out=out/boards/weact-bluepill-plus-ch32v203/template-smoke
cargo run --locked -q -p labwired-cli -- test \
  --script examples/weact-bluepill-plus-ch32v203/template-smoke.yaml \
  --output-dir "$out" --no-uart-stdout
python3 examples/weact-bluepill-plus-ch32v203/check-blink-rate.py "$out/result.json"
