#!/usr/bin/env bash
set -euo pipefail

# Trace drift assertion gate:
# - runs selected CI scripts
# - computes semantic fingerprints from result/snapshot/UART artifacts
# - compares against committed baseline hashes

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BASELINE_DIR="$CORE_DIR/examples/ci/fingerprints"

UPDATE_BASELINE=0
OUT_DIR="$CORE_DIR/out/trace-drift-assert"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --update-baseline)
      UPDATE_BASELINE=1
      shift
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

mkdir -p "$OUT_DIR"
mkdir -p "$BASELINE_DIR"

run_case() {
  local name="$1"
  local script_rel="$2"
  local expected_exit="$3"
  local case_dir="$OUT_DIR/$name"
  mkdir -p "$case_dir"

  echo "Running trace case: $name"
  set +e
  cargo run -q -p labwired-cli -- test \
    --script "$script_rel" \
    --no-uart-stdout \
    --output-dir "$case_dir"
  local rc=$?
  set -e
  if [[ "$rc" -ne "$expected_exit" ]]; then
    echo "Case '$name' exit $rc (expected $expected_exit)" >&2
    exit 1
  fi

  local result_json="$case_dir/result.json"
  local snapshot_json="$case_dir/snapshot.json"
  local uart_log="$case_dir/uart.log"
  local fingerprint_file="$case_dir/fingerprint.sha256"

  if [[ ! -f "$result_json" || ! -f "$snapshot_json" || ! -f "$uart_log" ]]; then
    echo "Missing artifacts for case '$name'" >&2
    exit 1
  fi

  local fp
  fp="$(python3 - "$result_json" "$snapshot_json" "$uart_log" <<'PY'
import hashlib
import json
import pathlib
import sys

result_path = pathlib.Path(sys.argv[1])
snapshot_path = pathlib.Path(sys.argv[2])
uart_path = pathlib.Path(sys.argv[3])

result = json.loads(result_path.read_text())
snapshot = json.loads(snapshot_path.read_text())
uart = uart_path.read_text()

payload = {
    "result": {
        "status": result.get("status"),
        "stop_reason": result.get("stop_reason"),
        "steps_executed": result.get("steps_executed"),
        "cycles": result.get("cycles"),
        "instructions": result.get("instructions"),
        "limits": result.get("limits"),
        "assertions": result.get("assertions"),
        "stop_reason_details": result.get("stop_reason_details"),
    },
    "cpu": snapshot.get("cpu"),
    "peripherals": snapshot.get("peripherals"),
    "uart": uart,
}

blob = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
print(hashlib.sha256(blob).hexdigest())
PY
)"

  echo "$fp" >"$fingerprint_file"

  local baseline="$BASELINE_DIR/$name.sha256"
  if [[ "$UPDATE_BASELINE" -eq 1 ]]; then
    cp "$fingerprint_file" "$baseline"
    echo "Updated baseline: $baseline"
  else
    if [[ ! -f "$baseline" ]]; then
      echo "Missing baseline: $baseline (run with --update-baseline)" >&2
      exit 1
    fi
    if ! cmp -s "$fingerprint_file" "$baseline"; then
      echo "Trace drift detected for case '$name'" >&2
      echo "Expected: $(cat "$baseline")" >&2
      echo "Actual:   $(cat "$fingerprint_file")" >&2
      exit 1
    fi
  fi
}

pushd "$CORE_DIR" >/dev/null
run_case "arm_uart_ok" "examples/ci/uart-ok.yaml" 0
run_case "riscv_uart_ok" "examples/ci/riscv-uart-ok.yaml" 0
run_case "arm_max_steps" "examples/ci/dummy-max-steps.yaml" 0
run_case "arm_memory_violation" "examples/ci/dummy-memory-violation.yaml" 0
popd >/dev/null

echo "Trace drift assertion complete."
