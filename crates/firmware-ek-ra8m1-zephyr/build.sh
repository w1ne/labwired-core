#!/usr/bin/env bash
# Rebuild the EK-RA8M1 Zephyr fixtures:
#   tests/fixtures/r7fa8m1ah-zephyr-hello.elf — samples/hello_world
#       (firmware_survival, examples/ek-ra8m1/zephyr-hello.yaml)
#   tests/fixtures/r7fa8m1ah-zephyr-sync.elf — samples/synchronization
#       (examples/ek-ra8m1/zephyr-sync.yaml: k_busy_wait on the SysTick
#       cycle counter, k_msleep woken by the SysTick system timer)
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

build_one() { # <sample-dir> <fixture-name>
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" -- \
    -DEXTRA_CFLAGS=-U__ARM_FEATURE_PAUTH -DEXTRA_AFLAGS=-U__ARM_FEATURE_PAUTH
  arm-none-eabi-strip --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 644 "$FIXTURES/$2"
  rm -rf "$build"
  arm-none-eabi-size "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world" "r7fa8m1ah-zephyr-hello.elf"
build_one "samples/synchronization" "r7fa8m1ah-zephyr-sync.elf"
