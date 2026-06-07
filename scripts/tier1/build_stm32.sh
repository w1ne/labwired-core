#!/usr/bin/env bash
# Rebuild the committed STM32 Tier-1 fixture blobs from source.
# Mirrors scripts/build_tier1_fixtures.sh, but for the nine STM32 silicon
# targets. Needs the rustup targets:
#   rustup target add thumbv6m-none-eabi thumbv7m-none-eabi thumbv7em-none-eabi
#
# MANIFEST.json is NOT refreshed here — run scripts/build_tier1_fixtures.sh
# (or the equivalent manifest step) after the blobs change.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/tests/fixtures/tier1"
mkdir -p "$OUT"

# chip -> thumb target. M33 parts (h563, wba52) build thumbv7m to match the
# in-repo H563 firmware convention (the simulator does not enforce ARMv8-M).
declare -A TARGETS=(
  [stm32f103]=thumbv7m-none-eabi
  [stm32f401]=thumbv7em-none-eabi
  [stm32f407]=thumbv7em-none-eabi
  [stm32g474re]=thumbv7em-none-eabi
  [stm32h563]=thumbv7m-none-eabi
  [stm32l073]=thumbv6m-none-eabi
  [stm32l476]=thumbv7em-none-eabi
  [stm32wb55]=thumbv7em-none-eabi
  [stm32wba52]=thumbv7m-none-eabi
)

build_chip() {
  local chip="$1"
  local target="${TARGETS[$chip]}"
  local src="$ROOT/examples/tier1-fixture/$chip"
  echo "==> $chip ($target)"
  (cd "$src" && cargo build --release --target "$target")
  cp "$src/target/$target/release/tier1-fixture-$chip" "$OUT/$chip.elf"
}

for chip in stm32f103 stm32f401 stm32f407 stm32g474re stm32h563 \
            stm32l073 stm32l476 stm32wb55 stm32wba52; do
  build_chip "$chip"
done

echo "STM32 Tier-1 fixture blobs refreshed in $OUT"
