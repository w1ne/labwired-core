#!/usr/bin/env bash
# Rebuild the NUCLEO-L552ZE-Q Zephyr fixtures:
#   tests/fixtures/stm32l552-zephyr-hello.elf — samples/hello_world
#     (firmware_survival, examples/nucleo-l552ze-q/zephyr-hello.yaml)
#   tests/fixtures/stm32l552-zephyr-sync.elf  — samples/synchronization
#     (examples/nucleo-l552ze-q/zephyr-sync.yaml)
#   tests/fixtures/stm32l552-zephyr-fpu-sharing.elf — tests/kernel/fpu_sharing/
#     generic, twister scenario kernel.fpu_sharing.generic.arm
#     (PI_NUM_ITERATIONS=70000); examples/nucleo-l552ze-q/zephyr-fpu-sharing.yaml
#     = Renode ARMv8M.robot "Should Pass Zephyr FPU Sharing Test"
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `nucleo_l552ze_q` (the TrustZone-disabled board target, so the image runs
# secure from 0x08000000). This is the same sample as Renode's ARMv8M.robot
# "Should Boot Zephyr Hello World", which loads the `_ns` build behind TF-M;
# LabWired does not model TrustZone, so it boots the non-TF-M board variant.
# The ELFs are debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis, hal_nxp, hal_stm32 and hal_ti modules.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="nucleo_l552ze_q"
FIXTURES="$HERE/../../tests/fixtures"

WEST="${WEST:-west}"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: $ZEPHYR_TOOLCHAIN_VARIANT ($(arm-none-eabi-gcc -dumpfullversion))"

# Pin the module list. West otherwise passes every module in the workspace,
# and each one adds an absolute CONFIG_ZEPHYR_<MOD>_MODULE symbol to the ELF
# symbol table (no code or data change), so a workspace that has since
# fetched hal_atmel/hal_renesas/... would produce different bytes. This is
# the set the hello fixture was first built with.
MODULES="$ZEPHYRPROJECT/modules/hal/cmsis;$ZEPHYRPROJECT/modules/hal/nxp;$ZEPHYRPROJECT/modules/hal/stm32;$ZEPHYRPROJECT/modules/hal/ti"

build_one() { # <sample-dir> <fixture-name> [extra cmake args...]
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" \
    -- "-DZEPHYR_MODULES=$MODULES" "${@:3}"
  arm-none-eabi-strip --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world"     "stm32l552-zephyr-hello.elf"
build_one "samples/synchronization" "stm32l552-zephyr-sync.elf"
build_one "tests/kernel/fpu_sharing/generic" "stm32l552-zephyr-fpu-sharing.elf" \
  -DPI_NUM_ITERATIONS=70000
