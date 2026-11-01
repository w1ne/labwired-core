#!/usr/bin/env bash
# Rebuild the EK-RA8M1 Zephyr fixtures:
#   tests/fixtures/r7fa8m1ah-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/ek-ra8m1/zephyr-hello.yaml)
#   tests/fixtures/r7fa8m1ah-zephyr-sync.elf — samples/synchronization
#       (examples/ek-ra8m1/zephyr-sync.yaml: k_busy_wait on the SysTick
#       cycle counter, k_msleep woken by the SysTick system timer)
#   tests/fixtures/r7fa8m1ah-zephyr-uart-basic-api.elf —
#       tests/drivers/uart/uart_basic_api (examples/ek-ra8m1/
#       zephyr-uart-basic-api.yaml: the interrupt-driven SCI_B driver, its
#       RXI/TXI/TEI linked to the NVIC through the ICU IELSRn, with the
#       driver's default receive FIFO)
#   tests/fixtures/r7fa8m1ah-zephyr-shell.elf —
#       samples/subsys/shell/shell_module (examples/ek-ra8m1/
#       zephyr-shell.yaml: the shell's interrupt-driven serial backend on
#       the same SCI_B RXI/TXI/TEI)
#   tests/fixtures/r7fa8m1ah-zephyr-counter-alarm.elf —
#       samples/drivers/counter/alarm from Zephyr v4.0.0 (see below;
#       examples/ek-ra8m1/zephyr-counter-alarm.yaml: AGT0 compare-match
#       alarms linked to the NVIC through the ICU IELSRn)
#   tests/fixtures/r7fa8m1ah-zephyr-button.elf — samples/basic/button from
#       Zephyr v4.1.0 (examples/ek-ra8m1/zephyr-button.yaml: S1 through the ICU
#       IRQ-pin half, PmnPFS.ISEL + IRQCRn, linked by IELSRn). v4.1.0 is the
#       first release whose RA GPIO driver supports pin interrupts
#       (renesas_ra_external_interrupt). Built UNMODIFIED from v4.1.0 (commit
#       7823374e8721; hal_renesas 3204903bdc5e, cmsis d1b8b20b6278) with the
#       module list pinned.
#   tests/fixtures/r7fa8m1ah-zephyr-blinky.elf — samples/basic/blinky from
#       Zephyr v4.1.0 (examples/ek-ra8m1/zephyr-blinky.yaml: LED1 made an
#       output through PmnPFS.PDR and toggled through PCNTR1.PODR), module
#       list pinned like the button.
#   tests/fixtures/r7fa8m1ah-zephyr-adc-api.elf — tests/drivers/adc/adc_api
#       from Zephyr v4.1.0 (examples/ek-ra8m1/zephyr-adc-api.yaml: ADC1
#       channels 0 and 2 through the ra_adc12 model, scan end linked by
#       IELSR39), module list pinned like the button.
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `ek_ra8m1`, with the hal_renesas module (Renesas FSP, BSD-3-Clause) at the
# revision Zephyr's west.yml pins (af77d7cdfeef). Zephyr compiles for
# `-mcpu=cortex-m85+nomve`, so GCC emits Armv8.1-M scalar instructions (the
# DLS/LE low-overhead loop in z_arm_interrupt_init, CSINC in cbprintf).
#
# One build-flag workaround, no source change: GCC >= 13 defines
# __ARM_FEATURE_PAUTH for cortex-m85, and the CMSIS 5.9 core_cm85.h that
# Zephyr v3.7 pins then includes pac_armv81.h, which that CMSIS revision
# does not ship. `-U__ARM_FEATURE_PAUTH` restores what the Zephyr SDK 0.16
# GCC 12 sees. No branch protection is enabled either way
# (-mbranch-protection stays at its default, none), so no PAC/BTI
# instruction is emitted.
#
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis and hal_renesas modules.
#   - For the counter alarm: a Zephyr v4.0.0 west workspace (ZEPHYRPROJECT_V4,
#     default ~/zephyrproject-v4.0) with cmsis and hal_renesas. v3.7.0 has
#     no Renesas AGT counter driver; v4.0.0 adds counter_renesas_ra_agt.c and
#     the in-tree samples/drivers/counter/alarm/boards/ek_ra8m1.overlay that
#     enables &agt0 (LOCO / 16 = 2048 Hz). Built UNMODIFIED from v4.0.0
#     (commit 8469084dfae8; hal_renesas 10326518701e, cmsis 4b96cbb17467,
#     the revisions its west.yml pins), with the same -U__ARM_FEATURE_PAUTH
#     flags, and the module list pinned so a workspace that fetched more
#     modules still rebuilds the same bytes.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="ek_ra8m1"
FIXTURES="$HERE/../../tests/fixtures"

WEST="${WEST:-west}"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: $ZEPHYR_TOOLCHAIN_VARIANT ($(arm-none-eabi-gcc -dumpfullversion))"

build_one() { # <sample-dir> <fixture-name> [extra cmake args...]
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" -- \
    -DEXTRA_CFLAGS=-U__ARM_FEATURE_PAUTH -DEXTRA_AFLAGS=-U__ARM_FEATURE_PAUTH "${@:3}"
  arm-none-eabi-strip --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 644 "$FIXTURES/$2"
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world" "r7fa8m1ah-zephyr-hello.elf"
build_one "samples/synchronization" "r7fa8m1ah-zephyr-sync.elf"
# Module list pinned (as for the counter alarm) so a workspace that fetched
# more modules still rebuilds the same bytes.
build_one "tests/drivers/uart/uart_basic_api" "r7fa8m1ah-zephyr-uart-basic-api.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT/modules/hal/cmsis;$ZEPHYRPROJECT/modules/hal/renesas"
build_one "samples/subsys/shell/shell_module" "r7fa8m1ah-zephyr-shell.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT/modules/hal/cmsis;$ZEPHYRPROJECT/modules/hal/renesas"

# Zephyr v4.0.0: samples/drivers/counter/alarm (AGT0 + ICU).
ZEPHYRPROJECT_V4="${ZEPHYRPROJECT_V4:-$HOME/zephyrproject-v4.0}"
export ZEPHYR_BASE="$ZEPHYRPROJECT_V4/zephyr"
echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
build_one "samples/drivers/counter/alarm" "r7fa8m1ah-zephyr-counter-alarm.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V4/modules/hal/cmsis;$ZEPHYRPROJECT_V4/modules/hal/renesas"

# Zephyr v4.1.0: samples/basic/button (IRQ13 on P009 through the ICU).
ZEPHYRPROJECT_V41="${ZEPHYRPROJECT_V41:-$HOME/zephyrproject-v4.1}"
export ZEPHYR_BASE="$ZEPHYRPROJECT_V41/zephyr"
echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
build_one "samples/basic/button" "r7fa8m1ah-zephyr-button.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# samples/basic/blinky (LED1 P600: output via PmnPFS.PDR, toggled through
# PCNTR1.PODR), same v4.1.0 tree.
build_one "samples/basic/blinky" "r7fa8m1ah-zephyr-blinky.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# tests/drivers/adc/adc_api (ADC1 channels 0 and 2 per the in-tree
# boards/ek_ra8m1.overlay; scan end ADC1_SCAN_END through IELSR39), same
# v4.1.0 tree.
build_one "tests/drivers/adc/adc_api" "r7fa8m1ah-zephyr-adc-api.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
