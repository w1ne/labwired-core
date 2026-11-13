#!/usr/bin/env bash
# Rebuild the MIMXRT595-EVK Zephyr fixtures:
#   tests/fixtures/mimxrt595-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/mimxrt595-evk/zephyr-hello.yaml)
#   tests/fixtures/mimxrt595-zephyr-sync.elf — samples/synchronization
#       (examples/mimxrt595-evk/zephyr-sync.yaml: k_busy_wait on the OS timer
#       count, k_msleep woken by the OS timer match IRQ)
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `mimxrt595_evk/mimxrt595s/cm33`, with the hal_nxp module (MCUXpresso SDK,
# BSD-3-Clause) at the revision Zephyr's west.yml pins (862e001504bd). The board
# runs secure: XIP from the FlexSPI secure alias 0x1800_0000, data at
# 0x3018_0000, peripherals at 0x5xxx_xxxx (see configs/chips/mimxrt595s.yaml).
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis and hal_nxp modules.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="mimxrt595_evk/mimxrt595s/cm33"
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

build_one "samples/hello_world" "mimxrt595-zephyr-hello.elf"
build_one "samples/synchronization" "mimxrt595-zephyr-sync.elf"
