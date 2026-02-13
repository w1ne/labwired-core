#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CORE_DIR="$(cd "$EXAMPLE_DIR/../.." && pwd)"

MODE="all" # emulator|hardware|all
UART_PORT="${UART_PORT:-}"
UART_TIMEOUT="${UART_TIMEOUT:-8}"
ARTIFACT_DIR="${ARTIFACT_DIR:-}"
KEEP_ARTIFACTS=0
LOG_LEVEL="${DEMO_LOG_LEVEL:-error}"

usage() {
  cat <<'EOF'
Usage:
  run_video_demo.sh [--mode emulator|hardware|all] [--port /dev/ttyACM0] [--timeout 8] [--artifacts-dir /path] [--keep-artifacts]

Examples:
  run_video_demo.sh --mode emulator
  run_video_demo.sh --mode all --port /dev/ttyACM0
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="${2:-}"
      shift 2
      ;;
    --port)
      UART_PORT="${2:-}"
      shift 2
      ;;
    --timeout)
      UART_TIMEOUT="${2:-}"
      shift 2
      ;;
    --artifacts-dir)
      ARTIFACT_DIR="${2:-}"
      shift 2
      ;;
    --keep-artifacts)
      KEEP_ARTIFACTS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$MODE" in
  emulator|hardware|all) ;;
  *)
    echo "error: --mode must be one of emulator|hardware|all" >&2
    exit 2
    ;;
esac

if [[ -z "$ARTIFACT_DIR" ]]; then
  ARTIFACT_DIR="$(mktemp -d /tmp/labwired-h563-video.XXXXXX)"
else
  mkdir -p "$ARTIFACT_DIR"
fi

cleanup() {
  if [[ "$KEEP_ARTIFACTS" != "1" ]]; then
    rm -rf "$ARTIFACT_DIR"
  fi
}
trap cleanup EXIT

run() {
  echo
  echo "==> $*"
  "$@"
}

run_quiet() {
  local label="$1"
  local log_file="$2"
  shift 2
  echo
  echo "==> $label"
  if ! "$@" >"$log_file" 2>&1; then
    echo "error: step failed: $label" >&2
    echo "log: $log_file" >&2
    tail -n 120 "$log_file" >&2 || true
    exit 1
  fi
}

require_pass_status() {
  local result_json="$1"
  if ! grep -q '"status": "pass"' "$result_json"; then
    echo "error: test status is not pass: $result_json" >&2
    exit 1
  fi
}

show_match() {
  local token="$1"
  local file="$2"
  local line
  line="$(grep -m1 -F "$token" "$file" || true)"
  if [[ -z "$line" ]]; then
    echo "error: token '$token' not found in $file" >&2
    exit 1
  fi
  echo "$line"
}

run_smoke() {
  local name="$1"
  local script_path="$2"
  shift 2
  local out_dir="$ARTIFACT_DIR/$name"
  local runner_log="$out_dir/runner.log"

  mkdir -p "$out_dir"
  echo
  echo "==> env RUST_LOG=$LOG_LEVEL cargo run -q -p labwired-cli -- test --script $script_path --no-uart-stdout --output-dir $out_dir"
  if ! env RUST_LOG="$LOG_LEVEL" cargo run -q -p labwired-cli -- test \
    --script "$script_path" \
    --no-uart-stdout \
    --output-dir "$out_dir" >"$runner_log" 2>&1; then
    echo "error: smoke run failed ($name). full log: $runner_log" >&2
    tail -n 120 "$runner_log" >&2 || true
    exit 1
  fi

  require_pass_status "$out_dir/result.json"
  echo "PASS: $name"
  local token
  for token in "$@"; do
    echo "  $(show_match "$token" "$out_dir/uart.log")"
  done
}

cd "$CORE_DIR"

echo "LabWired NUCLEO-H563ZI Video Demo"
echo "Mode: $MODE"
echo "Artifacts: $ARTIFACT_DIR"

if [[ "$MODE" == "emulator" || "$MODE" == "all" ]]; then
  run_quiet \
    "cargo test -p labwired-core test_flash_boot_alias_read_and_write -- --nocapture" \
    "$ARTIFACT_DIR/bootstrap-test.log" \
    cargo test -p labwired-core test_flash_boot_alias_read_and_write -- --nocapture
  echo "PASS: flash boot alias check"

  run_quiet \
    "cargo build --release --target thumbv7m-none-eabi -p firmware-h563-demo -p firmware-h563-io-demo -p firmware-h563-fullchip-demo" \
    "$ARTIFACT_DIR/bootstrap-build.log" \
    cargo build --release --target thumbv7m-none-eabi \
    -p firmware-h563-demo \
    -p firmware-h563-io-demo \
    -p firmware-h563-fullchip-demo
  echo "PASS: demo firmware build set"

  run_smoke "uart-smoke" \
    "examples/nucleo-h563zi/uart-smoke.yaml" \
    "OK"

  run_smoke "io-smoke" \
    "examples/nucleo-h563zi/io-smoke.yaml" \
    "H563-IO" \
    "PB0=1 PF4=1 PG4=1" \
    "PB0=0 PF4=0 PG4=0"

  run_smoke "fullchip-smoke" \
    "examples/nucleo-h563zi/fullchip-smoke.yaml" \
    "H563-FULLCHIP" \
    "RCC=1 SYSTICK=1 UART=1" \
    "ALL=1"
fi

if [[ "$MODE" == "hardware" || "$MODE" == "all" ]]; then
  HARDWARE_ARGS=(--timeout "$UART_TIMEOUT")
  if [[ -n "$UART_PORT" ]]; then
    HARDWARE_ARGS+=(--port "$UART_PORT")
  fi
  run "$SCRIPT_DIR/run_blink_uart_hardware.sh" "${HARDWARE_ARGS[@]}"
fi

echo
echo "Video demo run completed."
echo "Artifacts kept at: $ARTIFACT_DIR"
