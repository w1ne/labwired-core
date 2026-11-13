#!/usr/bin/env bash
# Rebuild the Apollo4 Plus EVB Zephyr fixture:
#   tests/fixtures/amap42kp-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/apollo4p-evb/zephyr-hello.yaml)
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `apollo4p_evb`, with the hal_ambiq module (AmbiqSuite SDK 4.4 HAL,
# BSD-3-Clause) at the revision Zephyr's west.yml pins (e25327f026df).
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis and hal_ambiq modules.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="apollo4p_evb"
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
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world" "amap42kp-zephyr-hello.elf"
