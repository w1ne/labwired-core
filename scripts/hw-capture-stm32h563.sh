#!/usr/bin/env bash
# Capture STM32H563 silicon state for LabWired sim validation.
#
# Run this on the laptop with a NUCLEO-H563ZI connected via its onboard
# STLINK-V3 (USB). Dumps reset-state register values into
# fixtures/stm32h563/hw-capture-<timestamp>/ for later diffing against the
# simulator (configs/chips/stm32h563.yaml).
#
# Prereq (Linux): apt install openocd   (>= 0.12.0)
#
# OpenOCD 0.12.0 ships no target/stm32h5x.cfg and its hla transport cannot
# select a debug AP > 0, while the H5's Cortex-M33 sits behind AP1. The
# working recipe (verified on a NUCLEO-H563ZI, STLINK V3J13M4) is the
# stlink-dap interface + dapdirect_swd + a hand-rolled cortex_m target on
# -ap-num 1. DP IDCODE is 0x6ba02477 (Cortex-M33 SW-DP).
#
# Usage:
#   bash scripts/hw-capture-stm32h563.sh
#
# Register addresses follow STMicroelectronics cmsis-device-h5
# Include/stm32h563xx.h (non-secure aliases — TZEN disabled) and RM0481.

set -euo pipefail

CORE_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${CORE_ROOT}/fixtures/stm32h563/hw-capture-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

SCR="/usr/share/openocd/scripts"
if command -v brew &>/dev/null; then
  BREW_SCR="$(brew --prefix openocd 2>/dev/null || true)/share/openocd/scripts"
  [ -d "$BREW_SCR" ] && SCR="$BREW_SCR"
fi
[ -d "$SCR" ] || { echo "ERROR: OpenOCD scripts not found at $SCR"; exit 1; }

echo "==> OpenOCD scripts: $SCR"
echo "==> Output dir:      $OUT"

# ─── Target config: STLINK-V3 dapdirect SWD, Cortex-M33 on AP1 ──────────────
cat > "$OUT/stm32h563-dap.cfg" <<'EOF'
source [find interface/stlink-dap.cfg]
transport select dapdirect_swd
source [find target/swj-dp.tcl]
adapter speed 1000
swj_newdap stm32h563 cpu -expected-id 0x6ba02477
dap create stm32h563.dap -chain-position stm32h563.cpu
target create stm32h563.cpu cortex_m -dap stm32h563.dap -ap-num 1
reset_config srst_only srst_nogate
EOF

# ─── Registers captured at reset (before any clock enables) ──────────────────
# Identity, Cortex-M33 system state, and the full RCC reset surface. RCC and
# the system block are always clocked, so these reads are valid immediately
# after `reset halt` with zero instructions executed.
RESET_REGS=(
  "DBGMCU_IDCODE       0x44024000 1"
  "CPUID               0xE000ED00 1"
  "ICSR                0xE000ED04 1"
  "VTOR                0xE000ED08 1"
  "AIRCR               0xE000ED0C 1"
  "SysTick             0xE000E010 4"
  "UID                 0x08FFF800 3"
  "FLASHSIZE           0x08FFF80C 1"
  "RCC_CR              0x44020C00 1"
  "RCC_HSICFGR         0x44020C10 1"
  "RCC_CSICFGR         0x44020C18 1"
  "RCC_CFGR1           0x44020C1C 1"
  "RCC_CFGR2           0x44020C20 1"
  "RCC_PLLCFGR_1_2_3   0x44020C28 3"
  "RCC_AHB1ENR         0x44020C88 1"
  "RCC_AHB2ENR         0x44020C8C 1"
  "RCC_APB1LENR        0x44020C9C 1"
  "RCC_APB1HENR        0x44020CA0 1"
  "RCC_APB2ENR         0x44020CA4 1"
  "RCC_APB3ENR         0x44020CA8 1"
  "RCC_BDCR            0x44020CF0 1"
  "RCC_RSR             0x44020CF4 1"
  "FLASH_ACR           0x40022000 8"
  "PWR                 0x44020800 8"
  "EXTI                0x44022000 6"
  "EXTI_IMR1_EMR1      0x44022080 2"
  "ICACHE              0x40030400 2"
  "IWDG                0x40003000 4"
)

# ─── Peripheral reset values (dumped after clock enables) ─────────────────────
# On the H5 a debugger read of a clock-gated peripheral does not return its
# reset value, so the capture enables the peripheral *bus clocks* first.
# Enabling an RCC bus clock does not alter any peripheral's own reset state.
# IWDG is intentionally NOT touched beyond reads (no KR writes — starting the
# watchdog is irreversible until reset). WWDG only arms when software sets
# CR.WDGA, so enabling its APB clock is safe.
PERIPH_REGS=(
  "GPIOA               0x42020000 10"
  "GPIOB               0x42020400 10"
  "GPIOC               0x42020800 10"
  "GPIOD               0x42020C00 10"
  "GPIOE               0x42021000 10"
  "GPIOF               0x42021400 10"
  "GPIOG               0x42021800 10"
  "USART1              0x40013800 8"
  "USART2              0x40004400 8"
  "USART3              0x40004800 8"
  "LPUART1             0x44002400 8"
  "TIM1                0x40012C00 12"
  "TIM2                0x40000000 12"
  "TIM3                0x40000400 12"
  "TIM6                0x40001000 12"
  "SPI1                0x40013000 6"
  "SPI2                0x40003800 6"
  "SPI3                0x40003C00 6"
  "I2C1                0x40005400 7"
  "I2C2                0x40005800 7"
  "GPDMA1              0x40020000 5"
  "GPDMA1_CH0          0x40020050 8"
  "GPDMA2              0x40021000 5"
  "ADC1                0x42028000 4"
  "RNG                 0x420C0800 2"
  "CRC                 0x40023000 3"
  "WWDG                0x40002C00 2"
  "RTC                 0x44007800 5"
  "LPTIM1              0x44004400 6"
)

# Clock-enable masks (cmsis-device-h5 RCC_*ENR bit positions):
#   AHB1ENR : GPDMA1|GPDMA2|CRC
#   AHB2ENR : GPIOA..G|ADC|RNG
#   APB1LENR: TIM2|TIM3|TIM6|WWDG|SPI2|SPI3|USART2|USART3|I2C1|I2C2
#   APB2ENR : TIM1|SPI1|USART1
#   APB3ENR : LPUART1|LPTIM1|RTCAPB
ENABLES=(
  "mww 0x44020C88 0x00001003"
  "mww 0x44020C8C 0x0004047F"
  "mww 0x44020C9C 0x0066C813"
  "mww 0x44020CA4 0x00005800"
  "mww 0x44020CA8 0x00200840"
)

OCD_CMDS=("init" "reset halt")
OCD_CMDS+=("echo {== identity + system + RCC at reset halt ==}")
for line in "${RESET_REGS[@]}"; do
  read -r name addr count <<<"$line"
  OCD_CMDS+=("echo {\"$name\": \"$addr\"}")
  OCD_CMDS+=("mdw $addr $count")
done
OCD_CMDS+=("echo {== enabling peripheral bus clocks ==}")
OCD_CMDS+=("${ENABLES[@]}")
OCD_CMDS+=("echo {== peripheral reset values (bus clocks on) ==}")
for line in "${PERIPH_REGS[@]}"; do
  read -r name addr count <<<"$line"
  OCD_CMDS+=("echo {\"$name\": \"$addr\"}")
  OCD_CMDS+=("mdw $addr $count")
done
# Leave the chip in a clean state: a real reset reverts the clock enables.
OCD_CMDS+=("reset run" "exit")

ARGS=(-f "$OUT/stm32h563-dap.cfg")
for c in "${OCD_CMDS[@]}"; do
  ARGS+=(-c "$c")
done

echo "==> Capturing registers"
# OpenOCD logs everything (including mdw output) on stderr.
openocd "${ARGS[@]}" >"$OUT/registers.txt" 2>&1

echo "==> Probe / target info:"
grep -E "STLINK|Target voltage|Cortex-M33|DPIDR" "$OUT/registers.txt" | tee "$OUT/probe-info.txt"

echo "==> Done: $OUT/registers.txt"
