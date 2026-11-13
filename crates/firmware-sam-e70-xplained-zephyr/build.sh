#!/usr/bin/env bash
# Rebuild the SAM E70 Xplained Zephyr fixtures:
#   tests/fixtures/same70q21b-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, same70_zephyr_boot, examples/sam-e70-xplained/zephyr-hello.yaml)
#   tests/fixtures/same70q21b-zephyr-sync.elf  — samples/synchronization
#       (examples/sam-e70-xplained/zephyr-sync.yaml)
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `sam_e70_xplained/same70q21b` (the revision-B silicon on current boards).
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis and hal_atmel modules.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="sam_e70_xplained/same70q21b"
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

build_one "samples/hello_world"     "same70q21b-zephyr-hello.elf"
build_one "samples/synchronization" "same70q21b-zephyr-sync.elf"
