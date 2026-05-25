#!/bin/bash
# ESP32-WROOM-32 HW oracle capture (chip-model verification, issue #105).
#
# Drives OpenOCD against a real ESP32-WROOM-32 connected via an ESP-Prog
# (FT2232H) JTAG adapter and records a baseline trace that the sim can be
# diff'd against by `esp32_replay_in_sim`.
#
# Captured signals (per "what we can sample without a logic analyzer"):
#   - PC samples at fixed wall-clock intervals (halt → reg pc → resume).
#   - Memory dumps of fixed checkpoint regions at the start and end of
#     the run (IRAM head, DRAM .bss head, GPIO+UART0 MMIO windows).
#   - UART0 stdout stream is NOT captured here (separate `tio` recording —
#     see README_esp32.md), because OpenOCD can't passively snoop UART.
#   - SPI bus snoop is INTENTIONALLY out of scope without a logic analyzer.
#
# Output layout (matches the existing hw-oracle naming convention):
#   scripts/hw-oracle/captures/esp32-wroom/<utc-ts>/
#     ├── oracle.json        Manifest: chip, sample params, region map.
#     ├── pc_trace.tsv       step\tns_offset\tpc_hex      (one line per sample)
#     ├── mem_pre.json       {addr_hex: [word_hex, ...]}  (pre-resume snapshot)
#     ├── mem_post.json      {addr_hex: [word_hex, ...]}  (post-cycles snapshot)
#     ├── openocd.log        Raw OpenOCD stdout/stderr (debugging).
#     └── elf.path           Absolute path of the firmware ELF flashed.
#
# Usage:
#   scripts/hw-oracle/esp32_capture.sh <firmware.elf> [opts]
#
# Options (env-var driven so the script stays argument-free for CI):
#   ESP32_OPENOCD_IF   OpenOCD interface cfg     (default: interface/ftdi/esp32_devkitj_v1.cfg)
#   ESP32_OPENOCD_TGT  OpenOCD target cfg        (default: target/esp32.cfg)
#   ESP32_SAMPLES      Number of PC samples      (default: 256)
#   ESP32_SAMPLE_MS    Interval between samples  (default: 20 ms)
#   ESP32_CAPTURE_DIR  Override capture root     (default: scripts/hw-oracle/captures/esp32-wroom)
#
# Exit codes:
#   0   Capture succeeded.
#   2   OpenOCD not installed.
#   3   No ESP32 detected (OpenOCD failed to attach within 30 s).
#   4   Bad arguments (missing/unreadable ELF).
#
# This script is safe to invoke in headless CI: it returns the gracefully-
# degraded exit codes above instead of hanging when no hardware is present.

set -u
set -o pipefail

# ── arg parsing ───────────────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
  echo "usage: $0 <firmware.elf>" >&2
  echo "       (see header of $0 for env-var options)" >&2
  exit 4
fi
ELF="$1"
if [[ ! -r "$ELF" ]]; then
  echo "error: firmware ELF '$ELF' is not readable" >&2
  exit 4
fi
ELF_ABS="$(readlink -f "$ELF")"

OOCD_IF="${ESP32_OPENOCD_IF:-interface/ftdi/esp32_devkitj_v1.cfg}"
OOCD_TGT="${ESP32_OPENOCD_TGT:-target/esp32.cfg}"
SAMPLES="${ESP32_SAMPLES:-256}"
SAMPLE_MS="${ESP32_SAMPLE_MS:-20}"
CAPTURE_ROOT="${ESP32_CAPTURE_DIR:-$(dirname "$0")/captures/esp32-wroom}"

# ── tool presence ─────────────────────────────────────────────────────────────
if ! command -v openocd >/dev/null 2>&1; then
  cat >&2 <<EOM
[esp32_capture] openocd not found on PATH.

Install ESP-IDF's fork (recommended; ships ESP32 configs):
  sudo apt install openocd                  # distro build, may lack esp32.cfg
  # or: https://github.com/espressif/openocd-esp32/releases

Then re-run:
  $0 $*
EOM
  exit 2
fi

# ── output dir ────────────────────────────────────────────────────────────────
TS="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="$CAPTURE_ROOT/$TS"
mkdir -p "$OUT"
echo "$ELF_ABS" > "$OUT/elf.path"
echo "[esp32_capture] capture dir: $OUT"

# ── checkpoint regions ─────────────────────────────────────────────────────────
# Word-addressed (32-bit). Counts chosen to keep the snapshot small while
# covering the windows most likely to diverge between sim and HW.
#
# Source: ESP32 TRM v4.9 §1.3.2 (address map) — same map our
# `configure_xtensa_esp32` builder targets.
#
# Format: "label addr_hex count"
CHECKPOINTS=(
  "iram_head            0x40080000   64"   # SRAM0 IRAM start (256 B)
  "dram_head            0x3FFAE000   64"   # SRAM2 DRAM start (256 B)
  "sram1_top            0x3FFFE000   16"   # SRAM1 high (initial stack)
  "uart0_regs           0x3FF40000   16"   # UART0 status/fifo window
  "gpio_out             0x3FF44000   16"   # GPIO output regs
  "rtc_apb_freq         0x3FF480B0    1"   # XTAL probe word
)

# ── helper: run an openocd batch script ────────────────────────────────────────
run_oocd() {
  local script="$1"
  local logfile="$2"
  # `-c "init; ...; shutdown"` keeps the daemon ephemeral. We pipe a tcl
  # script via a here-doc command file so multi-line sequencing is reliable.
  openocd \
    -f "$OOCD_IF" \
    -f "$OOCD_TGT" \
    -c "init" \
    -c "$script" \
    -c "shutdown" \
    >"$logfile" 2>&1
}

# ── attach probe ──────────────────────────────────────────────────────────────
echo "[esp32_capture] probing for ESP32 via openocd ($OOCD_IF + $OOCD_TGT)..."
if ! timeout 30 run_oocd "reset halt" "$OUT/openocd_probe.log"; then
  echo "[esp32_capture] openocd failed to attach. Tail of log:" >&2
  tail -n 40 "$OUT/openocd_probe.log" >&2 || true
  echo "" >&2
  echo "[esp32_capture] No hardware? See $(dirname "$0")/README_esp32.md" >&2
  # Still write a manifest so callers can detect "ran but no HW".
  cat > "$OUT/oracle.json" <<EOF
{
  "schema": "labwired-hw-oracle/esp32-wroom/v1",
  "status": "no_hardware",
  "elf": "$ELF_ABS",
  "captured_utc": "$TS",
  "openocd_interface": "$OOCD_IF",
  "openocd_target": "$OOCD_TGT"
}
EOF
  exit 3
fi

# ── flash the firmware ────────────────────────────────────────────────────────
echo "[esp32_capture] flashing $ELF_ABS..."
if ! run_oocd "reset halt; program $ELF_ABS verify; reset halt" "$OUT/openocd_flash.log"; then
  echo "[esp32_capture] flash failed. Tail of log:" >&2
  tail -n 60 "$OUT/openocd_flash.log" >&2 || true
  exit 1
fi

# ── pre-run memory snapshot ───────────────────────────────────────────────────
emit_mem_snapshot() {
  local out_json="$1"
  local oocd_log="$2"
  # Build a TCL batch that emits `LBW_MEM <addr_hex> <hex...>` lines we can
  # post-parse without depending on OpenOCD's verbose `mdw` formatting.
  local script=""
  for cp in "${CHECKPOINTS[@]}"; do
    # shellcheck disable=SC2206
    local parts=($cp)
    local addr="${parts[1]}"
    local count="${parts[2]}"
    # `mem2array dst width addr count` (width=32 → 32-bit words)
    script+="array unset words; mem2array words 32 $addr $count;"
    script+=" for {set i 0} {\$i < $count} {incr i} {"
    script+="   set off [expr {\$i * 4}];"
    script+="   puts [format \"LBW_MEM 0x%08x 0x%08x\" [expr {$addr + \$off}] \$words(\$i)];"
    script+=" };"
  done
  run_oocd "halt; $script" "$oocd_log" || true
  # Convert log → JSON {addr: word}. Python is the path of least resistance;
  # if not present we fall back to a jq-free hand-rolled dump.
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$oocd_log" "$out_json" <<'PY'
import json, sys
log, out = sys.argv[1], sys.argv[2]
words = {}
with open(log) as f:
    for line in f:
        if not line.startswith("LBW_MEM "):
            continue
        _, addr, val = line.split()
        words[addr] = val
with open(out, "w") as f:
    json.dump(words, f, indent=2, sort_keys=True)
PY
  else
    {
      echo "{"
      first=1
      grep '^LBW_MEM ' "$oocd_log" | while read -r _ addr val; do
        if [[ $first -eq 1 ]]; then first=0; else echo ","; fi
        printf '  "%s": "%s"' "$addr" "$val"
      done
      echo
      echo "}"
    } > "$out_json"
  fi
}

echo "[esp32_capture] capturing pre-run memory snapshot..."
emit_mem_snapshot "$OUT/mem_pre.json" "$OUT/openocd_mem_pre.log"

# ── PC sample loop (resume / halt / read pc / repeat) ─────────────────────────
echo "[esp32_capture] sampling PC: $SAMPLES samples every ${SAMPLE_MS} ms..."
printf 'step\tns_offset\tpc_hex\n' > "$OUT/pc_trace.tsv"
T0_NS=$(date +%s%N)
# Construct a single TCL script that does the full loop server-side so the
# round-trip overhead doesn't dominate the sample interval.
SAMPLE_SCRIPT="reset run;"
for ((i=0; i<SAMPLES; i++)); do
  SAMPLE_SCRIPT+=" after $SAMPLE_MS; halt;"
  # Print PC tagged with our sample index so we can recover step ordering.
  SAMPLE_SCRIPT+=" puts [format \"LBW_PC %d 0x%08x\" $i [reg pc -force]];"
  SAMPLE_SCRIPT+=" resume;"
done
run_oocd "$SAMPLE_SCRIPT" "$OUT/openocd_pc.log" || true
T1_NS=$(date +%s%N)
DUR_NS=$((T1_NS - T0_NS))

# Note: `reg pc -force` prints "pc (/32): 0x00000000" — we already wrap it
# in `puts [format ...]`, so the output line is literally "LBW_PC i 0xADDR".
# Convert that to the TSV. We approximate ns_offset as (i * SAMPLE_MS * 1e6)
# because OpenOCD doesn't expose per-sample wall-clock; this gives the
# replay tool a consistent x-axis even though it's nominal.
grep '^LBW_PC ' "$OUT/openocd_pc.log" | awk -v ms="$SAMPLE_MS" '
{
  step = $2
  pc = $3
  ns = step * ms * 1000000
  printf("%d\t%d\t%s\n", step, ns, pc)
}' >> "$OUT/pc_trace.tsv"

echo "[esp32_capture] PC trace: $(($(wc -l < "$OUT/pc_trace.tsv") - 1)) samples in $((DUR_NS / 1000000)) ms"

# ── post-run memory snapshot ──────────────────────────────────────────────────
echo "[esp32_capture] capturing post-run memory snapshot..."
emit_mem_snapshot "$OUT/mem_post.json" "$OUT/openocd_mem_post.log"

# ── consolidate openocd log ────────────────────────────────────────────────────
cat \
  "$OUT/openocd_probe.log" \
  "$OUT/openocd_flash.log" \
  "$OUT/openocd_mem_pre.log" \
  "$OUT/openocd_pc.log" \
  "$OUT/openocd_mem_post.log" \
  > "$OUT/openocd.log" 2>/dev/null || true
rm -f "$OUT"/openocd_{probe,flash,mem_pre,mem_post,pc}.log

# ── manifest ──────────────────────────────────────────────────────────────────
# Emit JSON without invoking jq — escape only the few strings that need it.
{
  echo "{"
  echo "  \"schema\": \"labwired-hw-oracle/esp32-wroom/v1\","
  echo "  \"status\": \"ok\","
  echo "  \"elf\": \"$ELF_ABS\","
  echo "  \"captured_utc\": \"$TS\","
  echo "  \"chip\": \"esp32-wroom-32\","
  echo "  \"cpu\": \"xtensa-lx6\","
  echo "  \"openocd_interface\": \"$OOCD_IF\","
  echo "  \"openocd_target\": \"$OOCD_TGT\","
  echo "  \"pc_samples\": $SAMPLES,"
  echo "  \"pc_sample_interval_ms\": $SAMPLE_MS,"
  echo "  \"capture_wall_ms\": $((DUR_NS / 1000000)),"
  echo "  \"checkpoints\": ["
  total=${#CHECKPOINTS[@]}
  i=0
  for cp in "${CHECKPOINTS[@]}"; do
    # shellcheck disable=SC2206
    parts=($cp)
    label="${parts[0]}"
    addr="${parts[1]}"
    count="${parts[2]}"
    sep=","
    if (( ++i == total )); then sep=""; fi
    echo "    { \"label\": \"$label\", \"addr\": \"$addr\", \"words\": $count }$sep"
  done
  echo "  ]"
  echo "}"
} > "$OUT/oracle.json"

echo "[esp32_capture] done: $OUT"
echo "[esp32_capture] next: cargo run --release -p labwired-hw-oracle --bin esp32_replay_in_sim -- \\"
echo "                        --capture $OUT --elf $ELF_ABS"
