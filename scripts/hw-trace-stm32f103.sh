#!/usr/bin/env bash
# Single-step trace of an STM32F103 firmware: capture (PC, R0..R15, xPSR,
# and key peripheral registers) at each step. Halts the chip via openocd
# and walks instructions one at a time, dumping state to JSONL.
#
# Use this on a connected board AFTER hw-capture-stm32f103.sh has dumped
# the reset state. Pair with sim-trace.sh + diff-trace.sh to find the
# exact instruction where the sim diverges from silicon.
#
# Usage:
#   bash core/scripts/hw-trace-stm32f103.sh <firmware.elf> [steps=200]
#
# Output: $OUT/trace.jsonl, one line per step, fields:
#   { step, pc, lr, sp, xpsr, r0..r12, flash_acr, rcc_cr, rcc_apb2enr,
#     gpioa_crl, gpioa_crh, gpioa_odr, usart1_sr, usart1_cr1, usart1_brr,
#     afio_mapr }

set -euo pipefail

FW="${1:?usage: $0 <firmware.elf> [steps]}"
STEPS="${2:-200}"
[ -f "$FW" ] || { echo "no firmware: $FW"; exit 1; }

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${REPO_ROOT}/core/fixtures/stm32f103/hw-trace-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
cp "$FW" "$OUT/firmware.elf"

SCR="$(brew --prefix open-ocd 2>/dev/null || brew --prefix openocd)/share/openocd/scripts"
[ -d "$SCR" ] || SCR="/usr/share/openocd/scripts"

echo "==> capturing $STEPS-step trace into $OUT"

# Watched peripheral registers (offsets in hex absolute).
PERIPHS=(
  "flash_acr        0x40022000"
  "rcc_cr           0x40021000"
  "rcc_cfgr         0x40021004"
  "rcc_apb1enr      0x4002101C"
  "rcc_apb2enr      0x40021018"
  "gpioa_crl        0x40010800"
  "gpioa_crh        0x40010804"
  "gpioa_odr        0x4001080C"
  "afio_mapr        0x40010004"
  "usart1_sr        0x40013800"
  "usart1_brr       0x40013808"
  "usart1_cr1       0x4001380C"
  "scb_vtor         0xE000ED08"
)

# Build an openocd command list. Halt → loop STEPS times: step, then
# `reg` for CPU regs, then mdw for each peripheral. Each iteration emits
# a marker line "STEP:i" then state lines.
OCMD=("init" "reset halt")
for ((i=0; i<STEPS; i++)); do
  OCMD+=("echo {step:$i}")
  OCMD+=("reg")
  for line in "${PERIPHS[@]}"; do
    IFS=' ' read -r name addr <<<"$line"
    OCMD+=("echo {$name:$addr}")
    OCMD+=("mdw $addr")
  done
  OCMD+=("step")
done
OCMD+=("resume" "exit")

# Flash the firmware first.
echo "==> flashing $FW"
openocd -s "$SCR" -f interface/stlink.cfg -f target/stm32f1x.cfg \
  -c "init" -c "reset halt" \
  -c "flash write_image erase $FW" \
  -c "reset halt" -c "exit" > "$OUT/flash.txt" 2>&1
tail -3 "$OUT/flash.txt"

# Build the step trace.
echo "==> running step trace ($STEPS steps)..."
OCD_ARGS=()
for c in "${OCMD[@]}"; do OCD_ARGS+=("-c" "$c"); done
openocd -s "$SCR" -f interface/stlink.cfg -f target/stm32f1x.cfg "${OCD_ARGS[@]}" \
  > "$OUT/raw-trace.txt" 2>&1 || true

echo "==> raw size: $(wc -l < "$OUT/raw-trace.txt") lines"
echo "    parse with: python3 core/scripts/parse-trace.py $OUT/raw-trace.txt > $OUT/trace.jsonl"

# Try the parser if it exists.
PARSER="$REPO_ROOT/core/scripts/parse-trace.py"
if [ -f "$PARSER" ]; then
  python3 "$PARSER" "$OUT/raw-trace.txt" > "$OUT/trace.jsonl" 2>&1 || true
  echo "==> JSONL: $(wc -l < "$OUT/trace.jsonl") records → $OUT/trace.jsonl"
fi
echo "==> Done. Capture: $OUT"
