#!/usr/bin/env bash
#
# F103 fidelity benchmark — LabWired silicon-fidelity regression suite.
#
# Runs three deliberately-broken firmware variants (plus a positive control) and
# checks that LabWired's verdict matches what real STM32F103 silicon does. The
# point: a faithful emulator must FAIL the firmware that real hardware fails. An
# emulator that passes a known-bad firmware gives a false pass — a green CI run
# that hides a shipping bug. This suite proves LabWired catches that class.
#
# A case PASSES iff its success marker (BENCH_UART_OK / BENCH_GPIO_OK /
# BENCH_RAM_OK) reaches the captured UART — the same signal a CI assertion uses.
#
# Exit status: 0 iff LabWired matches silicon ground truth on every case, so this
# script doubles as a CI fidelity regression guard.
#
# Env overrides:
#   LABWIRED_BIN  path to the labwired binary (default: auto-detect in ../../target)
#   BENCH_JSON    path for the machine-readable summary (default: ./benchmark-results.json)
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

if [[ -z "${LABWIRED_BIN:-}" ]]; then
  for cand in ../../target/debug/labwired ../../target/release/labwired; do
    [[ -x "$cand" ]] && LABWIRED_BIN="$cand" && break
  done
fi
: "${LABWIRED_BIN:?could not find labwired binary; build with 'cargo build -p labwired-cli' or set LABWIRED_BIN}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Building firmware variants..."
make -C firmware >/dev/null

# --- case table: name | smoke-script | success marker | silicon verdict -------
cases=(
  "control|control-smoke.yaml|BENCH_UART_OK|PASS"
  "clockbug|clockbug-smoke.yaml|BENCH_UART_OK|FAIL"
  "gpiobug|gpiobug-smoke.yaml|BENCH_GPIO_OK|FAIL"
  "rambug|rambug-smoke.yaml|BENCH_RAM_OK|FAIL"
)

verdict_from_marker() { grep -q "$2" "$1" 2>/dev/null && echo PASS || echo FAIL; }

run_labwired() { # <smoke-script> <marker> -> verdict
  local out="$work/lw"; rm -rf "$out"
  "$LABWIRED_BIN" test --script "./$1" --no-uart-stdout --output-dir "$out" >/dev/null 2>&1
  verdict_from_marker "$out/uart.log" "$2"
}

mark() { [[ "$1" == "$2" ]] && echo "ok" || echo "FALSE-PASS"; }

printf '\n%-10s %-12s %-18s\n' "case" "real-HW" "LabWired"
printf '%-10s %-12s %-18s\n'   "----" "-------" "--------"

correct=0; total=0; json_rows=()
for row in "${cases[@]}"; do
  IFS='|' read -r name script marker expected <<<"$row"
  lw=$(run_labwired "$script" "$marker")
  total=$((total+1)); ok=false; [[ "$lw" == "$expected" ]] && { correct=$((correct+1)); ok=true; }
  tag=$([[ "$lw" == "$expected" ]] && echo "$lw" || echo "$lw <$(mark "$lw" "$expected")>")
  printf '%-10s %-12s %-18s\n' "$name" "$expected" "$tag"
  json_rows+=("$(printf '{"case":"%s","expected":"%s","labwired":"%s","correct":%s}' \
    "$name" "$expected" "$lw" "$ok")")
done

BENCH_JSON="${BENCH_JSON:-benchmark-results.json}"
{
  printf '{\n  "board": "stm32f103",\n'
  printf '  "labwired_score": %d, "labwired_total": %d,\n' "$correct" "$total"
  printf '  "cases": [\n'
  for i in "${!json_rows[@]}"; do
    sep=,; [[ $i -eq $((${#json_rows[@]}-1)) ]] && sep=
    printf '    %s%s\n' "${json_rows[$i]}" "$sep"
  done
  printf '  ]\n}\n'
} > "$BENCH_JSON"

echo
printf 'fidelity score (verdicts matching real silicon): %d/%d\n' "$correct" "$total"
echo "wrote $BENCH_JSON"

if [[ $correct -ne $total ]]; then
  echo; echo "REGRESSION: LabWired disagrees with silicon ground truth."; exit 1
fi
echo; echo "LabWired matches silicon ground truth on all cases."
