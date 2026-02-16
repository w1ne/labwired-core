#!/usr/bin/env bash
set -euo pipefail

# Runs a firmware in LabWired and extracts unsupported instruction events
# (unknown 16-bit, unhandled 32-bit, unknown RISC-V) into audit artifacts.

usage() {
  cat <<'EOF'
Usage:
  core/scripts/unsupported_instruction_audit.sh \
    --firmware <path/to/elf> \
    [--system <path/to/system.yaml>] \
    [--max-steps <n>] \
    [--out-dir <path>] \
    [--fail-on-unsupported]

Outputs:
  - simulator.log
  - simulator.clean.log
  - run.json
  - metrics.json
  - report.md
  - unknown_thumb16_raw.txt
  - unknown_thumb16_summary.tsv
  - unhandled_thumb32_raw.txt
  - unhandled_thumb32_summary.tsv
  - unknown_riscv_raw.txt
  - unknown_riscv_summary.tsv
EOF
}

abs_path() {
  local p="$1"
  if [[ "$p" = /* ]]; then
    printf '%s\n' "$p"
  else
    printf '%s\n' "$PWD/$p"
  fi
}

count_lines() {
  local file="$1"
  if [[ -s "$file" ]]; then
    wc -l <"$file" | tr -d '[:space:]'
  else
    echo "0"
  fi
}

FIRMWARE=""
SYSTEM=""
MAX_STEPS=200000
OUT_DIR=""
FAIL_ON_UNSUPPORTED=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --firmware)
      FIRMWARE="${2:-}"
      shift 2
      ;;
    --system)
      SYSTEM="${2:-}"
      shift 2
      ;;
    --max-steps)
      MAX_STEPS="${2:-}"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="${2:-}"
      shift 2
      ;;
    --fail-on-unsupported)
      FAIL_ON_UNSUPPORTED=1
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

if [[ -z "$FIRMWARE" ]]; then
  echo "error: --firmware is required" >&2
  usage >&2
  exit 2
fi

if ! [[ "$MAX_STEPS" =~ ^[0-9]+$ ]]; then
  echo "error: --max-steps must be an integer" >&2
  exit 2
fi

if [[ ! -f "$FIRMWARE" ]]; then
  echo "error: firmware not found: $FIRMWARE" >&2
  exit 2
fi

if [[ -n "$SYSTEM" && ! -f "$SYSTEM" ]]; then
  echo "error: system manifest not found: $SYSTEM" >&2
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

FIRMWARE_ABS="$(abs_path "$FIRMWARE")"
SYSTEM_ABS=""
if [[ -n "$SYSTEM" ]]; then
  SYSTEM_ABS="$(abs_path "$SYSTEM")"
fi

if [[ -z "$OUT_DIR" ]]; then
  FW_TAG="$(basename "$FIRMWARE_ABS" | tr -c 'A-Za-z0-9._-' '_')"
  OUT_DIR="$CORE_DIR/out/unsupported-instruction-audit/$FW_TAG"
fi
OUT_DIR_ABS="$(abs_path "$OUT_DIR")"
mkdir -p "$OUT_DIR_ABS"

RAW_LOG="$OUT_DIR_ABS/simulator.log"
CLEAN_LOG="$OUT_DIR_ABS/simulator.clean.log"
RUN_JSON="$OUT_DIR_ABS/run.json"
REPORT_MD="$OUT_DIR_ABS/report.md"
METRICS_JSON="$OUT_DIR_ABS/metrics.json"

U16_RAW="$OUT_DIR_ABS/unknown_thumb16_raw.txt"
U16_SUMMARY="$OUT_DIR_ABS/unknown_thumb16_summary.tsv"
U32_RAW="$OUT_DIR_ABS/unhandled_thumb32_raw.txt"
U32_SUMMARY="$OUT_DIR_ABS/unhandled_thumb32_summary.tsv"
RV_RAW="$OUT_DIR_ABS/unknown_riscv_raw.txt"
RV_SUMMARY="$OUT_DIR_ABS/unknown_riscv_summary.tsv"

CMD=(cargo run -q -p labwired-cli -- --firmware "$FIRMWARE_ABS" --max-steps "$MAX_STEPS" --json)
if [[ -n "$SYSTEM_ABS" ]]; then
  CMD+=(--system "$SYSTEM_ABS")
fi

echo "Running unsupported-instruction audit..."
echo "  firmware: $FIRMWARE_ABS"
if [[ -n "$SYSTEM_ABS" ]]; then
  echo "  system:   $SYSTEM_ABS"
else
  echo "  system:   (default)"
fi
echo "  steps:    $MAX_STEPS"
echo "  out:      $OUT_DIR_ABS"

pushd "$CORE_DIR" >/dev/null
set +e
RUST_LOG=warn "${CMD[@]}" >"$RUN_JSON" 2>"$RAW_LOG"
SIM_EXIT=$?
set -e
popd >/dev/null

# Strip ANSI sequences before parsing.
sed -E $'s/\x1B\\[[0-9;]*[A-Za-z]//g' "$RAW_LOG" >"$CLEAN_LOG"

sed -nE 's/.*Unknown instruction at (0x[0-9a-fA-F]+): Opcode (0x[0-9a-fA-F]+).*/\2 \1/p' \
  "$CLEAN_LOG" >"$U16_RAW"
awk '
  {
    op = tolower($1);
    pc = tolower($2);
    count[op]++;
    if (!(op in sample_pc)) sample_pc[op] = pc;
  }
  END {
    for (op in count) {
      printf "%s\t%d\t%s\n", op, count[op], sample_pc[op];
    }
  }
' "$U16_RAW" | sort >"$U16_SUMMARY"

sed -nE 's/.*Internal: Unhandled 32-bit: ([0-9a-fA-F]+) ([0-9a-fA-F]+).*/\1 \2/p' \
  "$CLEAN_LOG" >"$U32_RAW"
awk '
  {
    h1 = tolower($1);
    h2 = tolower($2);
    key = h1 " " h2;
    count[key]++;
  }
  END {
    for (key in count) {
      split(key, parts, " ");
      printf "%s\t%s\t%d\n", parts[1], parts[2], count[key];
    }
  }
' "$U32_RAW" | sort >"$U32_SUMMARY"

sed -nE 's/.*Unknown instruction (0x[0-9a-fA-F]+) at (0x[0-9a-fA-F]+).*/\1 \2/p' \
  "$CLEAN_LOG" >"$RV_RAW"
awk '
  {
    op = tolower($1);
    pc = tolower($2);
    count[op]++;
    if (!(op in sample_pc)) sample_pc[op] = pc;
  }
  END {
    for (op in count) {
      printf "%s\t%d\t%s\n", op, count[op], sample_pc[op];
    }
  }
' "$RV_RAW" | sort >"$RV_SUMMARY"

U16_TOTAL="$(count_lines "$U16_RAW")"
U32_TOTAL="$(count_lines "$U32_RAW")"
RV_TOTAL="$(count_lines "$RV_RAW")"
UNSUPPORTED_TOTAL=$((U16_TOTAL + U32_TOTAL + RV_TOTAL))

# Parse executed instruction count from the final JSON line in run.json
INSTRUCTIONS_EXECUTED="$(python3 - "$RUN_JSON" <<'PY'
import json
import pathlib
import sys

p = pathlib.Path(sys.argv[1])
count = 0
for line in p.read_text().splitlines():
    line = line.strip()
    if not line.startswith("{") or "total_instructions" not in line:
        continue
    try:
        data = json.loads(line)
    except Exception:
        continue
    if isinstance(data, dict) and "total_instructions" in data:
        try:
            count = int(data["total_instructions"])
        except Exception:
            pass
print(count)
PY
)"

if ! [[ "$INSTRUCTIONS_EXECUTED" =~ ^[0-9]+$ ]]; then
  INSTRUCTIONS_EXECUTED=0
fi

SUPPORTED_INSTRUCTIONS=$((INSTRUCTIONS_EXECUTED - UNSUPPORTED_TOTAL))
if (( SUPPORTED_INSTRUCTIONS < 0 )); then
  SUPPORTED_INSTRUCTIONS=0
fi

if (( INSTRUCTIONS_EXECUTED > 0 )); then
  SUPPORT_PERCENT="$(awk -v s="$SUPPORTED_INSTRUCTIONS" -v t="$INSTRUCTIONS_EXECUTED" 'BEGIN { printf "%.4f", (s*100.0)/t }')"
else
  SUPPORT_PERCENT="0.0000"
fi

cat >"$METRICS_JSON" <<EOF
{
  "firmware": "$FIRMWARE_ABS",
  "system": "${SYSTEM_ABS:-}",
  "max_steps": $MAX_STEPS,
  "sim_exit_code": $SIM_EXIT,
  "instructions_executed": $INSTRUCTIONS_EXECUTED,
  "unsupported_total": $UNSUPPORTED_TOTAL,
  "supported_instructions": $SUPPORTED_INSTRUCTIONS,
  "instruction_support_percent": $SUPPORT_PERCENT,
  "unknown_thumb16_total": $U16_TOTAL,
  "unhandled_thumb32_total": $U32_TOTAL,
  "unknown_riscv_total": $RV_TOTAL
}
EOF

{
  echo "# Unsupported Instruction Audit Report"
  echo
  echo "- Firmware: \`$FIRMWARE_ABS\`"
  if [[ -n "$SYSTEM_ABS" ]]; then
    echo "- System: \`$SYSTEM_ABS\`"
  else
    echo "- System: (default)"
  fi
  echo "- Max steps: \`$MAX_STEPS\`"
  echo "- Simulator exit code: \`$SIM_EXIT\`"
  echo "- Instructions executed: \`$INSTRUCTIONS_EXECUTED\`"
  echo "- Unsupported observations (total): \`$UNSUPPORTED_TOTAL\`"
  echo "- Supported instructions: \`$SUPPORTED_INSTRUCTIONS\`"
  echo "- Instruction support coverage: \`$SUPPORT_PERCENT%\`"
  echo
  echo "## Unknown Thumb16 Instructions"
  if [[ -s "$U16_SUMMARY" ]]; then
    echo
    echo "| Opcode | Count | Sample PC |"
    echo "| --- | ---: | --- |"
    while IFS=$'\t' read -r opcode count sample_pc; do
      echo "| \`$opcode\` | $count | \`$sample_pc\` |"
    done <"$U16_SUMMARY"
  else
    echo
    echo "None detected."
  fi
  echo
  echo "## Unhandled Thumb32 Instructions"
  if [[ -s "$U32_SUMMARY" ]]; then
    echo
    echo "| H1 | H2 | Count |"
    echo "| --- | --- | ---: |"
    while IFS=$'\t' read -r h1 h2 count; do
      echo "| \`$h1\` | \`$h2\` | $count |"
    done <"$U32_SUMMARY"
  else
    echo
    echo "None detected."
  fi
  echo
  echo "## Unknown RISC-V Instructions"
  if [[ -s "$RV_SUMMARY" ]]; then
    echo
    echo "| Opcode | Count | Sample PC |"
    echo "| --- | ---: | --- |"
    while IFS=$'\t' read -r opcode count sample_pc; do
      echo "| \`$opcode\` | $count | \`$sample_pc\` |"
    done <"$RV_SUMMARY"
  else
    echo
    echo "None detected."
  fi
  echo
  echo "## Artifacts"
  echo
  echo "- \`$RAW_LOG\`"
  echo "- \`$CLEAN_LOG\`"
  echo "- \`$RUN_JSON\`"
  echo "- \`$REPORT_MD\`"
  echo "- \`$U16_SUMMARY\`"
  echo "- \`$U32_SUMMARY\`"
  echo "- \`$RV_SUMMARY\`"
} >"$REPORT_MD"

echo
echo "Audit summary:"
echo "  unknown_thumb16: $U16_TOTAL"
echo "  unhandled_thumb32: $U32_TOTAL"
echo "  unknown_riscv: $RV_TOTAL"
echo "  unsupported_total: $UNSUPPORTED_TOTAL"
echo "  report: $REPORT_MD"

EXIT_CODE="$SIM_EXIT"
if [[ "$FAIL_ON_UNSUPPORTED" -eq 1 && "$UNSUPPORTED_TOTAL" -gt 0 && "$EXIT_CODE" -eq 0 ]]; then
  EXIT_CODE=4
fi

exit "$EXIT_CODE"
