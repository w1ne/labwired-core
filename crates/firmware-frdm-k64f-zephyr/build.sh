#!/usr/bin/env bash
# Rebuild the FRDM-K64F Zephyr fixtures:
#   tests/fixtures/mk64f12-zephyr-hello.elf          — samples/hello_world
#       (firmware_survival, examples/frdm-k64f/zephyr-hello.yaml)
#   tests/fixtures/mk64f12-zephyr-uart-basic-api.elf — tests/drivers/uart/uart_basic_api
#       (examples/frdm-k64f/zephyr-uart-basic-api.yaml)
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `frdm_k64f`. uart_basic_api is the ztest suite Renode's NXP-K64F.robot
# "Should Run Zephyr Tests for UART" runs on this board: it types a line on
# UART0 after each "Please send characters to serial console" prompt and waits
# for "PROJECT EXECUTION SUCCESSFUL".
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
BOARD="frdm_k64f"
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

build_one "samples/hello_world"               "mk64f12-zephyr-hello.elf"
build_one "tests/drivers/uart/uart_basic_api" "mk64f12-zephyr-uart-basic-api.elf"
