#!/usr/bin/env bash
# Rebuild the nRF54L15 application-core Zephyr fixture ELF consumed by the
# firmware_survival / nrf54l15_zephyr_boot tests.
#
# The fixture is UNMODIFIED upstream Zephyr — the stock samples/hello_world built
# for board target nrf54l15dk/nrf54l15/cpuapp. We do not patch Zephyr; the whole
# point is that the simulator boots the vendor/community firmware as-is. The
# committed ELF (tests/fixtures/nrf54l15-zephyr-hello.elf) is the source of
# truth; this script reproduces it.
#
# Mirrors crates/firmware-nrf5340-zephyr/build.sh — same conventions, same
# toolchain assumptions.
#
# Requirements:
#   - A Zephyr west workspace (ZEPHYRPROJECT, default ~/zephyrproject) recent
#     enough to carry boards/nordic/nrf54l15dk.
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="nrf54l15dk/nrf54l15/cpuapp"
BUILD="${BUILD_DIR:-$HERE/build}"
FIXTURE="$HERE/../../tests/fixtures/nrf54l15-zephyr-hello.elf"

WEST="$ZEPHYRPROJECT/.venv/bin/west"
[ -x "$WEST" ] || WEST="west"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/opt/homebrew}"

echo "Zephyr:    $ZEPHYR_BASE"
echo "Board:     $BOARD"
echo "Toolchain: $ZEPHYR_TOOLCHAIN_VARIANT ($GNUARMEMB_TOOLCHAIN_PATH)"

"$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/samples/hello_world" -d "$BUILD"

arm-none-eabi-size "$BUILD/zephyr/zephyr.elf"
cp "$BUILD/zephyr/zephyr.elf" "$FIXTURE"
echo "Published $FIXTURE"
