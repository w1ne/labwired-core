#!/usr/bin/env bash
set -euo pipefail

# Fast digital-twin verification matrix for CI:
# - deterministic output (ARM + RISC-V repeated runs)
# - positive smoke checks
# - negative/fault-path checks

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

OUT_DIR="${1:-$CORE_DIR/out/digital-twin-verification}"
OUT_DIR_ABS="$OUT_DIR"
if [[ "$OUT_DIR_ABS" != /* ]]; then
  OUT_DIR_ABS="$PWD/$OUT_DIR_ABS"
fi

mkdir -p "$OUT_DIR_ABS"

run_case() {
  local name="$1"
  local script_rel="$2"
  local case_dir="$OUT_DIR_ABS/$name"
  mkdir -p "$case_dir"

  echo "Running case: $name"
  cargo run -q -p labwired-cli -- test \
    --script "$script_rel" \
    --no-uart-stdout \
    --output-dir "$case_dir"
}

run_case_expect_fail() {
  local name="$1"
  local script_rel="$2"
  local expected_exit="${3:-1}"
  local case_dir="$OUT_DIR_ABS/$name"
  mkdir -p "$case_dir"

  echo "Running expected-fail case: $name (expect exit $expected_exit)"
  set +e
  cargo run -q -p labwired-cli -- test \
    --script "$script_rel" \
    --no-uart-stdout \
    --output-dir "$case_dir"
  local rc=$?
  set -e

  if [[ "$rc" -ne "$expected_exit" ]]; then
    echo "Unexpected exit code for $name: got $rc, expected $expected_exit" >&2
    exit 1
  fi
}

pushd "$CORE_DIR" >/dev/null

run_case "arm_uart_ok_a" "examples/ci/uart-ok.yaml"
run_case "arm_uart_ok_b" "examples/ci/uart-ok.yaml"
run_case "riscv_uart_ok_a" "examples/ci/riscv-uart-ok.yaml"
run_case "riscv_uart_ok_b" "examples/ci/riscv-uart-ok.yaml"

run_case "arm_max_steps" "examples/ci/dummy-max-steps.yaml"
run_case "arm_max_uart_bytes" "examples/ci/dummy-max-uart-bytes.yaml"
run_case "arm_no_progress" "examples/ci/dummy-no-progress.yaml"
run_case "arm_memory_violation" "examples/ci/dummy-memory-violation.yaml"
run_case_expect_fail "arm_uart_assertion_fail" "examples/ci/dummy-fail-uart.yaml" 1

# Determinism checks (repeat-run JSON artifacts must match exactly).
cmp \
  "$OUT_DIR_ABS/arm_uart_ok_a/result.json" \
  "$OUT_DIR_ABS/arm_uart_ok_b/result.json"
cmp \
  "$OUT_DIR_ABS/riscv_uart_ok_a/result.json" \
  "$OUT_DIR_ABS/riscv_uart_ok_b/result.json"

popd >/dev/null

echo "Digital twin verification complete: $OUT_DIR_ABS"
