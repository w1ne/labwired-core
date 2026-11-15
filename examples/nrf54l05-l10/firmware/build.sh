#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
src="$root/examples/nrf54l05-l10/firmware"
mkdir -p "$root/tests/fixtures/board-smoke"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
CFLAGS="-mcpu=cortex-m33 -mthumb -mfloat-abi=soft -std=c11 -Os -Wall -Wextra -Wpedantic -Werror -ffreestanding -fno-common -ffunction-sections -fdata-sections"
for chip in nrf54l05 nrf54l10; do
  macro="$(printf '%s' "$chip" | tr '[:lower:]' '[:upper:]')"
  arm-none-eabi-gcc $CFLAGS -D"$macro" -c "$src/main.c" -o "$tmp/main.o"
  arm-none-eabi-gcc $CFLAGS -c "$root/examples/nrf54l15-dk/src/startup.c" -o "$tmp/startup.o"
  out="$root/tests/fixtures/board-smoke/$chip.elf"
  arm-none-eabi-gcc $CFLAGS "$tmp/main.o" "$tmp/startup.o" \
    -T "$src/$chip.ld" -nostdlib -nostartfiles \
    -Wl,--build-id=none -Wl,--gc-sections -o "$out"
  sha256sum "$out"
done
