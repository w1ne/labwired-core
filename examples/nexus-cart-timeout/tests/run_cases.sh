#!/usr/bin/env bash
set -euo pipefail
fixture="$(cd "$(dirname "$0")/.." && pwd)"
cli="$(printenv LABWIRED_CLI || echo labwired)"
make -C "$fixture"
python3 "$fixture/tests/build_case.py" >/dev/null
for case in stale missing malformed healthy no-start recovery; do
    output="$fixture/out/$case"
    mkdir -p "$output"
    "$cli" test --script "$fixture/tests/$case.yaml" \
      --output-dir "$output" --watch-gpio gpiob:0 --run-manifest --json \
      > "$output/runner.log" 2>&1 || {
        tail -50 "$output/runner.log"
        exit 1
      }
    python3 "$fixture/tests/verify_waveform.py" "$case" "$output/result.json"
done
