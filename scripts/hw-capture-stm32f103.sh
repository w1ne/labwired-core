#!/usr/bin/env bash
# Capture STM32F103 silicon state for LabWired sim validation.
#
# Run this on the laptop with the board connected via ST-LINK V2 (USB).
# Dumps reset-state register values + a firmware UART trace into
# fixtures/stm32f103/hw-capture-<timestamp>/ for later diffing against
# the simulator.
#
# Prereq (macOS): brew install openocd stlink
# Prereq (Linux): apt install openocd stlink-tools libusb-1.0-0-dev
#
# Usage:
#   bash core/scripts/hw-capture-stm32f103.sh [firmware.elf]
#
# If no firmware argument, only reset-state capture runs.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${REPO_ROOT}/core/fixtures/stm32f103/hw-capture-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

SCR="$(brew --prefix open-ocd 2>/dev/null || brew --prefix openocd 2>/dev/null)/share/openocd/scripts"
[ -d "$SCR" ] || SCR="/usr/share/openocd/scripts"

echo "==> OpenOCD scripts: $SCR"
echo "==> Output dir:      $OUT"

# ─── Step 1: probe ──────────────────────────────────────────────────────────
echo "==> st-info --probe"
st-info --probe > "$OUT/st-info.txt" 2>&1 || true
cat "$OUT/st-info.txt"

# ─── Step 2: read reset-state registers (halt, dump, resume) ─────────────────
# These are the registers Arduino HAL touches early in SystemInit / HAL_Init
# and Serial1.begin(). When the sim diverges from silicon, this baseline tells
# us which model is wrong.
REGS=(
  "DBGMCU_IDCODE       0xE0042000"
  "FLASH_ACR           0x40022000"
  "FLASH_KEYR          0x40022004"
  "FLASH_OPTKEYR       0x40022008"
  "FLASH_SR            0x4002200C"
  "FLASH_CR            0x40022010"
  "FLASH_OBR           0x4002201C"
  "RCC_CR              0x40021000"
  "RCC_CFGR            0x40021004"
  "RCC_CIR             0x40021008"
  "RCC_APB2RSTR        0x4002100C"
  "RCC_APB1RSTR        0x40021010"
  "RCC_AHBENR          0x40021014"
  "RCC_APB2ENR         0x40021018"
  "RCC_APB1ENR         0x4002101C"
  "RCC_BDCR            0x40021020"
  "RCC_CSR             0x40021024"
  "PWR_CR              0x40007000"
  "PWR_CSR             0x40007004"
  "AFIO_EVCR           0x40010000"
  "AFIO_MAPR           0x40010004"
  "AFIO_EXTICR1        0x40010008"
  "GPIOA_CRL           0x40010800"
  "GPIOA_CRH           0x40010804"
  "GPIOA_IDR           0x40010808"
  "GPIOA_ODR           0x4001080C"
  "GPIOA_BSRR          0x40010810"
  "GPIOA_LCKR          0x40010818"
  "USART1_SR           0x40013800"
  "USART1_DR           0x40013804"
  "USART1_BRR          0x40013808"
  "USART1_CR1          0x4001380C"
  "USART1_CR2          0x40013810"
  "USART1_CR3          0x40013814"
  "USART1_GTPR         0x40013818"
  "IWDG_KR             0x40003000"
  "IWDG_PR             0x40003004"
  "IWDG_RLR            0x40003008"
  "IWDG_SR             0x4000300C"
  "SCB_VTOR            0xE000ED08"
  "SCB_AIRCR           0xE000ED0C"
  "SCB_SCR             0xE000ED10"
  "SCB_CCR             0xE000ED14"
  "SysTick_CTRL        0xE000E010"
  "SysTick_LOAD        0xE000E014"
  "SysTick_VAL         0xE000E018"
  "SysTick_CALIB       0xE000E01C"
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
openocd -s "$SCR" -f interface/stlink.cfg -f target/stm32f1x.cfg "${OCD_ARGS[@]}" \
  > "$OUT/registers.txt" 2>&1 || true

echo "==> Baseline register dump → $OUT/registers.txt"
head -20 "$OUT/registers.txt"
echo "  ... (full file in $OUT/registers.txt)"

# ─── Step 3: optional firmware flash + UART capture ──────────────────────────
FIRMWARE="${1:-}"
if [ -n "$FIRMWARE" ] && [ -f "$FIRMWARE" ]; then
  echo ""
  echo "==> Flashing $FIRMWARE"
  OBJCOPY="$(find ~/.platformio/packages/toolchain-gccarmnoneeabi -name arm-none-eabi-objcopy 2>/dev/null | head -1)"
  [ -n "$OBJCOPY" ] || { echo "objcopy not found — install platformio toolchain"; exit 1; }
  cp "$FIRMWARE" "$OUT/firmware.elf"
  "$OBJCOPY" -O binary "$OUT/firmware.elf" "$OUT/firmware.bin"

  st-flash --connect-under-reset --reset write "$OUT/firmware.bin" 0x08000000 \
    > "$OUT/st-flash.txt" 2>&1 || true
  tail -5 "$OUT/st-flash.txt"

  # Locate ST-LINK V2 VCP. On macOS it shows up as /dev/cu.usbmodem*.
  PORT="$(ls /dev/cu.usbmodem* 2>/dev/null | head -1 || ls /dev/ttyACM* 2>/dev/null | head -1)"
  if [ -n "$PORT" ]; then
    echo "==> Capturing UART from $PORT @ 9600 8N1 for 15s"
    # macOS has no `timeout` — backgrounded cat + sleep + kill.
    stty -f "$PORT" 9600 cs8 -cstopb -parenb 2>/dev/null || stty -F "$PORT" 9600 cs8 -cstopb -parenb 2>/dev/null || true
    cat "$PORT" > "$OUT/uart.log" &
    CAT_PID=$!
    sleep 15
    kill $CAT_PID 2>/dev/null || true
    echo "==> Captured UART → $OUT/uart.log ($(wc -c < "$OUT/uart.log") bytes)"
    head -10 "$OUT/uart.log"
  else
    echo "==> No /dev/cu.usbmodem* or /dev/ttyACM* found; skipping UART capture."
  fi
fi

echo ""
echo "==> Done. Capture dir: $OUT"
echo "    Next step (sim diff): bash core/scripts/diff-sim-vs-hw.sh $OUT [firmware.elf]"
