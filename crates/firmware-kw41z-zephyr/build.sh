#!/usr/bin/env bash
# Rebuild the KW41Z Zephyr fixture ELFs consumed by firmware_survival:
#   tests/fixtures/kw41z-zephyr-hello.elf       — samples/hello_world
#   tests/fixtures/kw41z-zephyr-fxos8700.elf    — samples/sensor/fxos8700
#
# Both are UNMODIFIED upstream Zephyr built for board `frdm_kw41z`. We do not
# patch Zephyr; the whole point is that the simulator boots the stock
# vendor/community firmware as-is. The fxos8700 build is forced into polling
# mode (CONFIG_FXOS8700_TRIGGER_NONE=y) so it needs no GPIO data-ready interrupt
# line from the sensor — everything else (hybrid accel+mag, temperature) is the
# sample's own default config. The committed ELFs are the source of truth; this
# script reproduces them.
#
# Requirements:
#   - A Zephyr v3.7.x west workspace (ZEPHYRPROJECT, default ~/zephyrproject).
#   - arm-none-eabi-gcc on PATH (the gnuarmemb toolchain variant); no Zephyr SDK
#     install needed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject}"
BOARD="frdm_kw41z"
FIXTURES="$HERE/../../tests/fixtures"

WEST="$ZEPHYRPROJECT/.venv/bin/west"
[ -x "$WEST" ] || WEST="west"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT="${ZEPHYR_TOOLCHAIN_VARIANT:-gnuarmemb}"
export GNUARMEMB_TOOLCHAIN_PATH="${GNUARMEMB_TOOLCHAIN_PATH:-/usr}"

echo "Zephyr:    $ZEPHYR_BASE"
echo "Board:     $BOARD"
echo "Toolchain: $ZEPHYR_TOOLCHAIN_VARIANT ($GNUARMEMB_TOOLCHAIN_PATH)"

build_one() { # <sample-dir> <fixture-name> [extra -D args...]
  local sample="$1" fixture="$2"; shift 2
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$sample" -d "$build" "$@"
  arm-none-eabi-size "$build/zephyr/zephyr.elf"
  cp "$build/zephyr/zephyr.elf" "$FIXTURES/$fixture"
  rm -rf "$build"
  echo "Published $FIXTURES/$fixture"
}

build_one "samples/hello_world"       "kw41z-zephyr-hello.elf"
build_one "samples/sensor/fxos8700"   "kw41z-zephyr-fxos8700.elf" -- -DCONFIG_FXOS8700_TRIGGER_NONE=y
