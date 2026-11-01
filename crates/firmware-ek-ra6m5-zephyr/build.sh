#!/usr/bin/env bash
# Rebuild the EK-RA6M5 Zephyr fixtures:
#   tests/fixtures/r7fa6m5bh-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/ek-ra6m5/zephyr-hello.yaml)
#   tests/fixtures/r7fa6m5bh-zephyr-sync.elf — samples/synchronization
#       (examples/ek-ra6m5/zephyr-sync.yaml: k_busy_wait on the SysTick
#       cycle counter, k_msleep woken by the SysTick system timer)
#   tests/fixtures/r7fa6m5bh-zephyr-counter-alarm.elf —
#       samples/drivers/counter/alarm from Zephyr v4.1.0 (see below;
#       examples/ek-ra6m5/zephyr-counter-alarm.yaml: AGT0 compare-match
#       alarms linked to the NVIC through the ICU IELSRn)
#   tests/fixtures/r7fa6m5bh-zephyr-button.elf — samples/basic/button from
#       Zephyr v4.1.0 (examples/ek-ra6m5/zephyr-button.yaml: S1 through the ICU
#       IRQ-pin half, PmnPFS.ISEL + IRQCRn, linked by IELSRn). v4.1.0 is the
#       first release whose RA GPIO driver supports pin interrupts
#       (renesas_ra_external_interrupt). Built UNMODIFIED from v4.1.0 (commit
#       7823374e8721; hal_renesas 3204903bdc5e, cmsis d1b8b20b6278) with the
#       module list pinned.
#   tests/fixtures/r7fa6m5bh-zephyr-blinky.elf — samples/basic/blinky from
#       Zephyr v4.1.0 (examples/ek-ra6m5/zephyr-blinky.yaml: LED1 made an
#       output through PmnPFS.PDR and toggled through PCNTR1.PODR), module
#       list pinned like the button.
#   tests/fixtures/r7fa6m5bh-zephyr-adc-api.elf — tests/drivers/adc/adc_api
#       from Zephyr v4.1.0 (examples/ek-ra6m5/zephyr-adc-api.yaml: ADC1
#       channels 0 and 2 through the ra_adc12 model, scan end linked by
#       IELSR39), module list pinned like the button.
#   tests/fixtures/r7fa6m5bh-zephyr-bme280.elf — samples/sensor/bme280 from
#       Zephyr v4.1.0 (examples/ek-ra6m5/zephyr-bme280.yaml: a BME280 on
#       IIC1 through the ra_iic model, RXI/TXI/TEI/ERI linked by
#       IELSR91-94). Two build inputs beyond the module pin: bme280-iic1.overlay
#       (next to this script) puts the BME280 at 0x76 on &iic1 (the sample's
#       own overlays need an `arduino_i2c` label ek_ra6m5 lacks), and the
#       picolibc module (82d62ed1ac55, CONFIG_PICOLIBC_USE_MODULE): the RA IIC
#       driver calls ceil() and the BME280 decoder pow(), which Zephyr's
#       minimal libc lacks, and the distro newlib has no retargetable locking.
#       `west update picolibc` in the v4.1.0 workspace fetches it.
#
# UNMODIFIED upstream Zephyr v4.0.0 (commit 8469084dfae8) built for board
# `ek_ra6m5`, with the hal_renesas module (Renesas FSP, BSD-3-Clause) and
# cmsis at the revisions Zephyr's west.yml pins (10326518701e /
# 4b96cbb17467). Zephyr v3.7.0, which the other Zephyr fixtures in this repo
# use, has no `ek_ra6m5` board; v4.0.0 is the first release that does. No
# build-flag workaround is needed (the part is a plain Cortex-M33).
#
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v4.0.0 west workspace (ZEPHYRPROJECT, default
#     ~/zephyrproject-v4.0) with the cmsis and hal_renesas modules:
#       west init -m https://github.com/zephyrproject-rtos/zephyr --mr v4.0.0
#       west update cmsis hal_renesas
#   - For the counter alarm: a Zephyr v4.1.0 west workspace
#     (ZEPHYRPROJECT_V41, default ~/zephyrproject-v4.1) with cmsis and
#     hal_renesas. v4.0.0 has the AGT counter driver but no RA6 `agt` nodes;
#     v4.1.0 adds them to ra6-cm33-common.dtsi along with the in-tree
#     samples/drivers/counter/alarm/boards/ek_ra6m5.overlay that enables
#     &agt0 (LOCO / 16 = 2048 Hz, agtcmai on IELSR84). Built UNMODIFIED from
#     v4.1.0 (commit 7823374e8721; hal_renesas 3204903bdc5e, cmsis
#     d1b8b20b6278, the revisions its west.yml pins), with the module list
#     pinned so a workspace that fetched more modules still rebuilds the same
#     bytes.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject-v4.0}"
BOARD="ek_ra6m5"
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
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" -- "${@:3}"
  arm-none-eabi-strip --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 644 "$FIXTURES/$2"
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

# The v4.0.0 fixtures were first built with only cmsis and hal_renesas; pin
# that list so a workspace that has since fetched more modules (picolibc
# swaps the C library) still rebuilds the same bytes.
V40_MODULES="-DZEPHYR_MODULES=$ZEPHYRPROJECT/modules/hal/cmsis;$ZEPHYRPROJECT/modules/hal/renesas"
build_one "samples/hello_world" "r7fa6m5bh-zephyr-hello.elf" "$V40_MODULES"
build_one "samples/synchronization" "r7fa6m5bh-zephyr-sync.elf" "$V40_MODULES"

# Zephyr v4.1.0: samples/drivers/counter/alarm (AGT0 + ICU).
ZEPHYRPROJECT_V41="${ZEPHYRPROJECT_V41:-$HOME/zephyrproject-v4.1}"
export ZEPHYR_BASE="$ZEPHYRPROJECT_V41/zephyr"
echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
build_one "samples/drivers/counter/alarm" "r7fa6m5bh-zephyr-counter-alarm.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# samples/basic/button (IRQ10 on P005 through the ICU), same v4.1.0 tree.
build_one "samples/basic/button" "r7fa6m5bh-zephyr-button.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# samples/basic/blinky (LED1 P006: output via PmnPFS.PDR, toggled through
# PCNTR1.PODR), same v4.1.0 tree.
build_one "samples/basic/blinky" "r7fa6m5bh-zephyr-blinky.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# tests/drivers/adc/adc_api (ADC1 channels 0 and 2 per the in-tree
# boards/ek_ra6m5.overlay; scan end ADC1_SCAN_END 0x166 through IELSR39),
# same v4.1.0 tree.
build_one "tests/drivers/adc/adc_api" "r7fa6m5bh-zephyr-adc-api.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas"
# samples/sensor/bme280 (BME280 at 0x76 on IIC1, see the header), same
# v4.1.0 tree plus picolibc.
build_one "samples/sensor/bme280" "r7fa6m5bh-zephyr-bme280.elf" \
  "-DZEPHYR_MODULES=$ZEPHYRPROJECT_V41/modules/hal/cmsis;$ZEPHYRPROJECT_V41/modules/hal/renesas;$ZEPHYRPROJECT_V41/modules/lib/picolibc" \
  -DCONFIG_PICOLIBC_USE_MODULE=y "-DEXTRA_DTC_OVERLAY_FILE=$HERE/bme280-iic1.overlay"
