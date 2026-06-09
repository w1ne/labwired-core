#!/usr/bin/env bash
# Capture nRF52840 silicon state for LabWired sim validation.
#
# Run this on the laptop with the board connected via ST-LINK (USB).
# Dumps reset-state register values into fixtures/nrf52840/hw-capture-<timestamp>/
# for later diffing against the simulator.
#
# Prereq (macOS): brew install openocd stlink
# Prereq (Linux): apt install openocd stlink-tools libusb-1.0-0-dev
#
# Usage:
#   bash core/scripts/hw-capture-nrf52840.sh
#
# Hardware: Seeed XIAO nRF52840 Sense connected via ST-LINK + OpenOCD 0.12.0.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${REPO_ROOT}/core/fixtures/nrf52840/hw-capture-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

SCR=""
if command -v brew &>/dev/null; then
  SCR="$(brew --prefix openocd 2>/dev/null || true)/share/openocd/scripts"
  [ -d "$SCR" ] && SCR="$SCR" || SCR=""
fi
[ -z "$SCR" ] && SCR="/usr/share/openocd/scripts"
[ -d "$SCR" ] || { echo "ERROR: OpenOCD scripts not found at $SCR"; exit 1; }

echo "==> OpenOCD scripts: $SCR"
echo "==> Output dir:      $OUT"

# ─── Step 1: probe ──────────────────────────────────────────────────────────
echo "==> st-info --probe"
st-info --probe > "$OUT/st-info.txt" 2>&1 || true
cat "$OUT/st-info.txt"

# ─── Step 2: read reset-state registers (halt, dump, resume) ─────────────────
# These are the registers representing nRF52840 silicon identity, system state,
# and key peripheral reset values. When the sim diverges from silicon, this
# baseline tells us which model is wrong.
REGS=(
  "INFO.PART           0x10000100"
  "INFO.VARIANT        0x10000104"
  "DEVICEID[0]         0x10000060"
  "DEVICEID[1]         0x10000064"
  "SCB_VTOR            0xE000ED08"
  "SysTick_CTRL        0xE000E010"
  "TIMER0_BITMODE      0x40008508"
  "TIMER0_PRESCALER    0x40008510"
  "RTC0_PRESCALER      0x40011508"
  "WDT_RUNSTATUS       0x40010400"
  "WDT_CRV             0x40010504"
  "RNG_CONFIG          0x4000D504"
  "PWM0_COUNTERTOP     0x4001C548"
  "SAADC_RESOLUTION    0x40007510"
  "QSPI_IFCONFIG0      0x40029544"
  "COMP_TH             0x40013530"
  "QDEC_SAMPLEPER      0x40012508"
  "PDM_PDMCLKCTRL      0x4001D540"
  "GPIO0_OUT           0x50000504"
  "GPIO0_DIR           0x50000514"
  "GPIO1_OUT           0x50000804"
  "GPIO1_DIR           0x50000814"
  "UART0_ENABLE        0x40002500"
  "SPIM0_ENABLE        0x40003500"
  "NVMC_READY          0x4001E400"
  "USBD_ENABLE         0x40027500"
  "RADIO_FREQUENCY     0x40001508"
  "RADIO_MODE          0x40001510"
)

OCD_CMDS=("init" "reset halt")
for line in "${REGS[@]}"; do
  IFS=' ' read -r name addr <<<"$line"
  OCD_CMDS+=("echo {\"$name\": \"$addr\"}")
  OCD_CMDS+=("mdw $addr")
done
OCD_CMDS+=("resume" "exit")

OCD_ARGS=()
for c in "${OCD_CMDS[@]}"; do OCD_ARGS+=("-c" "$c"); done

echo "==> openocd: reset, halt, dump ${#REGS[@]} registers"
openocd -s "$SCR" -f interface/stlink.cfg -f target/nrf52.cfg "${OCD_ARGS[@]}" \
  > "$OUT/registers.txt" 2>&1 || true

echo "==> Baseline register dump → $OUT/registers.txt"
head -30 "$OUT/registers.txt"
echo "  ... (full file in $OUT/registers.txt)"

echo ""
echo "==> Done. Capture dir: $OUT"
echo "    Next step (sim diff): bash core/scripts/diff-sim-vs-hw.sh $OUT"
