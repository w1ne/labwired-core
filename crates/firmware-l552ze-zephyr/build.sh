#!/usr/bin/env bash
# Rebuild the NUCLEO-L552ZE-Q Zephyr fixture consumed by firmware_survival and
# examples/nucleo-l552ze-q/zephyr-hello.yaml:
#   tests/fixtures/stm32l552-zephyr-hello.elf — samples/hello_world
#
# UNMODIFIED upstream Zephyr v3.7.0 (commit 36940db938a8) built for board
# `nucleo_l552ze_q` (the TrustZone-disabled board target, so the image runs
# secure from 0x08000000). This is the same sample as Renode's ARMv8M.robot
# "Should Boot Zephyr Hello World", which loads the `_ns` build behind TF-M;
# LabWired does not model TrustZone, so it boots the non-TF-M board variant.
# The ELF is debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so the build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v3.7.0 west workspace (ZEPHYRPROJECT, default ~/zephyrproject)
#     with the cmsis and hal_stm32 modules.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="nucleo_l552ze_q"
FIXTURE="$HERE/../../tests/fixtures/stm32l552-zephyr-hello.elf"

WEST="${WEST:-west}"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: $ZEPHYR_TOOLCHAIN_VARIANT ($(arm-none-eabi-gcc -dumpfullversion))"

build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT
"$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/samples/hello_world" -d "$build"
arm-none-eabi-strip --strip-debug -o "$FIXTURE" "$build/zephyr/zephyr.elf"
arm-none-eabi-size "$FIXTURE"
sha256sum "$FIXTURE"
