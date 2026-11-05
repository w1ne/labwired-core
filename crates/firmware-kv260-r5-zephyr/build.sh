#!/usr/bin/env bash
# Rebuild the AMD Kria KV260 (Zynq UltraScale+ XCK26, RPU Cortex-R5F) Zephyr
# fixtures:
#   tests/fixtures/kv260-r5-zephyr-hello.elf         — samples/hello_world
#   tests/fixtures/kv260-r5-zephyr-sync.elf          — samples/synchronization
#   tests/fixtures/kv260-r5-zephyr-kernel-common.elf — tests/kernel/common
#
# UNMODIFIED upstream Zephyr v4.1.0 for board target `kv260_r5`: RPU core 0,
# linked at 0 (TCM/DDR low window), console on PS UART1, TTC0 system timer at
# the board DT's 5 MHz, RPU GIC (PL390).
# The ELFs are debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v4.1.0 west workspace (ZEPHYRPROJECT, default
#     ~/zephyrproject-v4.1) with the cmsis and picolibc modules.
#   - An arm-none-eabi GCC (the `gnuarmemb` toolchain variant; no Zephyr SDK
#     needed). Ubuntu 24.04 `gcc-arm-none-eabi` 13.2.1 built the fixtures.
#   - gperf on PATH (tests/kernel/common enables CONFIG_USERSPACE).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject-v4.1}"
BOARD="kv260_r5"
FIXTURES="$HERE/../../tests/fixtures"

WEST="${WEST:-west}"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT=gnuarmemb
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"
CROSS="$GNUARMEMB_TOOLCHAIN_PATH/bin/arm-none-eabi-"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: ${CROSS}gcc $("${CROSS}gcc" -dumpfullversion)"

# Pin the module list: west otherwise passes every module in the workspace,
# and each adds an absolute CONFIG_ZEPHYR_<MOD>_MODULE symbol to the ELF.
MODULES="$ZEPHYRPROJECT/modules/hal/cmsis;$ZEPHYRPROJECT/modules/lib/picolibc"

build_one() { # <sample-dir> <fixture-name> [extra cmake args...]
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" \
    -- "-DZEPHYR_MODULES=$MODULES" "${@:3}"
  "${CROSS}strip" --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 0644 "$FIXTURES/$2"
  rm -rf "$build"
  "${CROSS}size" "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world"     "kv260-r5-zephyr-hello.elf"
build_one "samples/synchronization" "kv260-r5-zephyr-sync.elf"
build_one "tests/kernel/common"     "kv260-r5-zephyr-kernel-common.elf"
