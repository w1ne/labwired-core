#!/usr/bin/env bash
#
# F103 fidelity benchmark runner.
#
# Runs three firmware variants (control / clockbug / rambug) on LabWired and, if
# available, on Renode, and diffs each engine's verdict against the silicon
# ground truth. An emulator is "faithful" when its verdict matches real hardware
# on every case; a verdict that passes a known-bad firmware is a FALSE PASS.
#
# Verdict signal is unified across engines: a case PASSES iff its success marker
# (BENCH_UART_OK / BENCH_RAM_OK) appears in the captured UART. That is what a CI
# assertion keys on, and it is the only signal Renode exposes, so both engines
# are judged identically.
#
# Exit status: 0 iff LabWired matches ground truth on every case (so this script
# doubles as a CI regression guard for LabWired's fidelity). Renode mismatches
# are reported but do not fail the run — they are the point of the benchmark.
#
# Env overrides:
#   LABWIRED_BIN  path to the labwired binary (default: auto-detect in ../../target)
#   RENODE_BIN    path to the renode launcher (default: `renode` on PATH; if not
#                 found, the Renode column is skipped)
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$here"

# --- locate engines ----------------------------------------------------------
if [[ -z "${LABWIRED_BIN:-}" ]]; then
  for cand in ../../target/debug/labwired ../../target/release/labwired; do
    [[ -x "$cand" ]] && LABWIRED_BIN="$cand" && break
  done
fi
: "${LABWIRED_BIN:?could not find labwired binary; build with 'cargo build -p labwired-cli' or set LABWIRED_BIN}"

RENODE_BIN="${RENODE_BIN:-$(command -v renode || true)}"
REPL="${RENODE_REPL:-}"          # optional override of the .repl path
have_renode=0
[[ -n "$RENODE_BIN" && -x "$RENODE_BIN" ]] && have_renode=1

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# --- build firmware ----------------------------------------------------------
echo "Building firmware variants..."
make -C firmware >/dev/null

# --- case table: name | smoke-script | firmware | success marker | expected --
#   expected is the real-silicon verdict (PASS = correct firmware behaviour).
cases=(
  "control|control-smoke.yaml|control.elf|BENCH_UART_OK|PASS"
  "clockbug|clockbug-smoke.yaml|clockbug.elf|BENCH_UART_OK|FAIL"
  "gpiobug|gpiobug-smoke.yaml|gpiobug.elf|BENCH_GPIO_OK|FAIL"
  "rambug|rambug-smoke.yaml|rambug.elf|BENCH_RAM_OK|FAIL"
)

# verdict_from_marker <file> <marker> -> echoes PASS or FAIL
verdict_from_marker() {
  grep -q "$2" "$1" 2>/dev/null && echo PASS || echo FAIL
}

run_labwired() { # <smoke-script> <marker> -> verdict
  local out="$work/lw"; rm -rf "$out"
  "$LABWIRED_BIN" test --script "./$1" \
    --no-uart-stdout --output-dir "$out" >/dev/null 2>&1
  verdict_from_marker "$out/uart.log" "$2"
}

run_renode() { # <firmware> <marker> -> verdict
  local elf="$here/firmware/build/$1" out="$work/rn-$1.txt"
  local repl="$REPL"
  if [[ -z "$repl" ]]; then
    repl="$(dirname "$RENODE_BIN")/platforms/cpus/stm32f103.repl"
    [[ -f "$repl" ]] || repl="$(dirname "$RENODE_BIN")/../platforms/cpus/stm32f103.repl"
  fi
  if [[ ! -f "$repl" ]]; then echo "NO-REPL"; return; fi
  local resc="$work/run-$1.resc"
  sed -e "s#@REPL@#$repl#" -e "s#@ELF@#$elf#" -e "s#@OUT@#$out#" \
    "$here/renode/run.resc.template" > "$resc"
  rm -f "$out"
  timeout 180 "$RENODE_BIN" --disable-xwt --console -e "include @$resc" >/dev/null 2>&1
  verdict_from_marker "$out" "$2"
}

# --- run + tabulate ----------------------------------------------------------
mark() { [[ "$1" == "$2" ]] && echo "ok" || echo "FALSE-PASS"; }

printf '\n%-10s %-12s %-18s %-18s\n' "case" "real-HW" "LabWired" "Renode"
printf '%-10s %-12s %-18s %-18s\n' "----" "-------" "--------" "------"

lw_correct=0; lw_total=0; rn_correct=0; rn_total=0
json_rows=()
for row in "${cases[@]}"; do
  IFS='|' read -r name script fw marker expected <<<"$row"
  lw=$(run_labwired "$script" "$marker")
  lw_total=$((lw_total+1)); lw_ok=false; [[ "$lw" == "$expected" ]] && { lw_correct=$((lw_correct+1)); lw_ok=true; }
  lw_tag=$([[ "$lw" == "$expected" ]] && echo "$lw" || echo "$lw <$(mark "$lw" "$expected")>")
  rn="skipped"; rn_ok="null"
  if [[ $have_renode -eq 1 ]]; then
    rn=$(run_renode "$fw" "$marker")
    if [[ "$rn" == "NO-REPL" || "$rn" == "" ]]; then
      rn_tag="(no f103 repl)"; rn="no-platform"
    else
      rn_total=$((rn_total+1)); rn_ok=false; [[ "$rn" == "$expected" ]] && { rn_correct=$((rn_correct+1)); rn_ok=true; }
      rn_tag=$([[ "$rn" == "$expected" ]] && echo "$rn" || echo "$rn <$(mark "$rn" "$expected")>")
    fi
  else
    rn_tag="(skipped)"
  fi
  printf '%-10s %-12s %-18s %-18s\n' "$name" "$expected" "$lw_tag" "$rn_tag"
  json_rows+=("$(printf '{"case":"%s","expected":"%s","labwired":"%s","labwired_correct":%s,"renode":"%s","renode_correct":%s}' \
    "$name" "$expected" "$lw" "$lw_ok" "$rn" "$rn_ok")")
done

echo
echo "fidelity score (verdicts matching real silicon):"
printf '  LabWired: %d/%d\n' "$lw_correct" "$lw_total"
if [[ $have_renode -eq 1 && $rn_total -gt 0 ]]; then
  printf '  Renode:   %d/%d\n' "$rn_correct" "$rn_total"
else
  echo "  Renode:   not run (set RENODE_BIN to a renode launcher to include it)"
fi

# --- machine-readable summary (for CI / dashboards) --------------------------
BENCH_JSON="${BENCH_JSON:-benchmark-results.json}"
{
  printf '{\n  "board": "stm32f103",\n'
  printf '  "labwired_score": %d, "labwired_total": %d,\n' "$lw_correct" "$lw_total"
  if [[ $have_renode -eq 1 && $rn_total -gt 0 ]]; then
    printf '  "renode_score": %d, "renode_total": %d,\n' "$rn_correct" "$rn_total"
  else
    printf '  "renode_score": null, "renode_total": null,\n'
  fi
  printf '  "cases": [\n'
  for i in "${!json_rows[@]}"; do
    sep=,; [[ $i -eq $((${#json_rows[@]}-1)) ]] && sep=
    printf '    %s%s\n' "${json_rows[$i]}" "$sep"
  done
  printf '  ]\n}\n'
} > "$BENCH_JSON"
echo "wrote $BENCH_JSON"

if [[ $lw_correct -ne $lw_total ]]; then
  echo; echo "REGRESSION: LabWired disagrees with silicon ground truth."; exit 1
fi
echo; echo "LabWired matches silicon ground truth on all cases."
