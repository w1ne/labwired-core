#!/usr/bin/env bash
# Rebuild the nRF5340 application-core Zephyr fixture ELF consumed by the
# firmware_survival / nrf5340_clock_boot tests.
#
# The fixture is UNMODIFIED upstream Zephyr — the stock samples/hello_world built
# for board target nrf5340dk/nrf5340/cpuapp. We do not patch Zephyr; the whole
# point is that the simulator boots the vendor/community firmware as-is. The
# committed ELF (tests/fixtures/nrf5340-zephyr-hello.elf) is the source of truth;
# this script reproduces it.
#
# Requirements:
#   - A Zephyr v3.7.x west workspace (ZEPHYRPROJECT, default ~/zephyrproject).
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="nrf5340dk/nrf5340/cpuapp"
BUILD="${BUILD_DIR:-$HERE/build}"
FIXTURE="$HERE/../../tests/fixtures/nrf5340-zephyr-hello.elf"

WEST="$ZEPHYRPROJECT/.venv/bin/west"
[ -x "$WEST" ] || WEST="west"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"

echo "Zephyr:   $ZEPHYR_BASE"
echo "Board:    $BOARD"
echo "Toolchain:$ZEPHYR_TOOLCHAIN_VARIANT ($GNUARMEMB_TOOLCHAIN_PATH)"

"$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/samples/hello_world" -d "$BUILD"

arm-none-eabi-size "$BUILD/zephyr/zephyr.elf"
cp "$BUILD/zephyr/zephyr.elf" "$FIXTURE"
echo "Published $FIXTURE"
