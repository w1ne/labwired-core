#!/usr/bin/env bash
# Rebuild the EK-RA6M5 Zephyr fixtures:
#   tests/fixtures/r7fa6m5bh-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/ek-ra6m5/zephyr-hello.yaml)
#   tests/fixtures/r7fa6m5bh-zephyr-sync.elf — samples/synchronization
#       (examples/ek-ra6m5/zephyr-sync.yaml: k_busy_wait on the SysTick
#       cycle counter, k_msleep woken by the SysTick system timer)
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

build_one() { # <sample-dir> <fixture-name>
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build"
  arm-none-eabi-strip --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 644 "$FIXTURES/$2"
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world" "r7fa6m5bh-zephyr-hello.elf"
build_one "samples/synchronization" "r7fa6m5bh-zephyr-sync.elf"
