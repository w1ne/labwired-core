#!/usr/bin/env bash
# Run the full coverage matrix locally and generate a scoreboard.
#
# Usage:
#   ./scripts/run_coverage_matrix.sh [--out-dir DIR] [--skip-build] [--targets TARGET,...]
#
# Mirrors the CI workflow (core-coverage-matrix-smoke.yml) so local results
# match what CI would produce.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

OUT_DIR="${REPO_ROOT}/out/coverage-matrix"
SKIP_BUILD=false
FILTER_TARGETS=""
REQUIRED_TARGETS="stm32f103-bluepill,stm32h563-nucleo,stm32f401-nucleo,firmware-rv32i-ci-fixture,firmware-stm32f103-blinky-stm32f103"
MIN_PASS_RATE="0.80"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)   OUT_DIR="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=true; shift ;;
    --targets)   FILTER_TARGETS="$2"; shift 2 ;;
    --min-pass-rate) MIN_PASS_RATE="$2"; shift 2 ;;
    -h|--help)
      echo "Usage: $0 [--out-dir DIR] [--skip-build] [--targets id1,id2,...] [--min-pass-rate 0.80]"
      exit 0
      ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Matrix definition -- single source of truth for local runs.
# Format: id|target|crate|script|system
MATRIX=(
  "ci-fixture-armv6m|thumbv6m-none-eabi|firmware-armv6m-ci-fixture|examples/ci/uart-ok.yaml|configs/systems/ci-fixture-uart1.yaml"
  "firmware-rv32i-ci-fixture|riscv32i-unknown-none-elf|firmware-rv32i-ci-fixture|examples/ci/riscv-uart-ok.yaml|configs/systems/ci-fixture-riscv-uart1.yaml"
  "firmware-stm32f103-blinky-stm32f103|thumbv7m-none-eabi|firmware-stm32f103-blinky|examples/firmware-stm32f103-blinky/io-smoke.yaml|examples/firmware-stm32f103-blinky/system.yaml"
  "stm32h563-nucleo|thumbv7m-none-eabi|firmware-h563-demo|examples/nucleo-h563zi/uart-smoke.yaml|examples/nucleo-h563zi/system.yaml"
  "stm32f401-nucleo|thumbv7em-none-eabi|firmware-f401-demo|examples/nucleo-f401re/uart-smoke.yaml|configs/systems/nucleo-f401re.yaml"
  "stm32f103-bluepill|thumbv7m-none-eabi|firmware-stm32f103-blinky|examples/firmware-stm32f103-blinky/io-smoke.yaml|examples/firmware-stm32f103-blinky/system.yaml"
  "stm32f401-blackpill|thumbv7em-none-eabi|firmware-f401-demo|examples/blackpill-f401cc/uart-smoke.yaml|examples/blackpill-f401cc/system.yaml"
  "nrf52832-example|thumbv7em-none-eabi|firmware-nrf52832-demo|examples/nrf52832/uart-smoke.yaml|configs/systems/nrf52832-example.yaml"
  "rp2040-pico|thumbv6m-none-eabi|firmware-rp2040-pio-onboarding|examples/rp2040-pio/asm-smoke.yaml|examples/rp2040-pio/system-asm.yaml"
)

cd "${REPO_ROOT}"

pass=0
fail=0
skip=0
total=0

if [[ "${SKIP_BUILD}" == "false" ]]; then
  echo "==> Building CLI..."
  cargo build --release -p labwired-cli --quiet
fi

for entry in "${MATRIX[@]}"; do
  IFS='|' read -r id target crate script system <<< "${entry}"

  if [[ -n "${FILTER_TARGETS}" ]] && ! echo ",${FILTER_TARGETS}," | grep -q ",${id},"; then
    continue
  fi

  total=$((total + 1))
  target_dir="${OUT_DIR}/${id}"
  mkdir -p "${target_dir}"

  echo ""
  echo "==> [${total}] ${id}"

  if [[ "${SKIP_BUILD}" == "false" ]]; then
    echo "    Building ${crate} (${target})..."
    if ! cargo build -p "${crate}" --release --target "${target}" --quiet 2>"${target_dir}/build.log"; then
      echo "    FAIL (build)"
      fail=$((fail + 1))
      continue
    fi
  fi

  echo "    Running smoke test..."
  if cargo run --release -q -p labwired-cli -- test \
       --script "${script}" \
       --output-dir "${target_dir}" \
       --no-uart-stdout 2>"${target_dir}/smoke.log"; then
    :
  fi

  if [[ -f "${target_dir}/result.json" ]]; then
    status=$(python3 -c "import json; print(json.load(open('${target_dir}/result.json'))['status'])")
    if [[ "${status}" == "pass" ]]; then
      echo "    PASS"
      pass=$((pass + 1))
    else
      echo "    FAIL (status: ${status})"
      fail=$((fail + 1))
    fi
  else
    echo "    FAIL (no result.json)"
    fail=$((fail + 1))
  fi
done

echo ""
echo "============================================"
echo "Coverage Matrix Results: ${pass}/${total} pass, ${fail} fail"
echo "============================================"

# Generate scoreboard
required_args=""
IFS=',' read -ra REQ_LIST <<< "${REQUIRED_TARGETS}"
for rt in "${REQ_LIST[@]}"; do
  required_args="${required_args} --required-target ${rt}"
done

python3 scripts/generate_coverage_matrix_scoreboard.py \
  --matrix-root "${OUT_DIR}" \
  --markdown-out "${OUT_DIR}/scoreboard.md" \
  --json-out "${OUT_DIR}/scoreboard.json" \
  ${required_args} \
  --min-required-pass-rate "${MIN_PASS_RATE}"
gate_exit=$?

echo ""
echo "Scoreboard written to: ${OUT_DIR}/scoreboard.md"
echo "JSON report written to: ${OUT_DIR}/scoreboard.json"

if [[ "${gate_exit}" -ne 0 ]]; then
  echo ""
  echo "ERROR: Release gate check FAILED (exit ${gate_exit})"
  exit "${gate_exit}"
fi

echo ""
echo "Release gate check PASSED (required pass rate >= ${MIN_PASS_RATE})"
