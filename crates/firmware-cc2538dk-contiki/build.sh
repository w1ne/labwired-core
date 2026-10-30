#!/usr/bin/env bash
# Rebuild the CC2538DK Contiki-NG fixtures (TARGET=cc2538dk, full default
# netstack: RPL Lite, 6LoWPAN, CSMA on the cc2538 RF driver). The committed
# ELFs are the source of truth; this script reproduces them.
#   tests/fixtures/cc2538dk-contiki-hello.elf       UNMODIFIED examples/hello-world
#   tests/fixtures/cc2538dk-contiki-test-cfs.elf    UNMODIFIED examples/storage/cfs-coffee test-cfs
#   tests/fixtures/cc2538dk-contiki-coffee.elf      UNMODIFIED examples/storage/cfs-coffee example-coffee
#   tests/fixtures/cc2538dk-contiki-flash-test.elf  flash-test/ (ours, MIT)
# The three newer ones are --strip-debug'ed so no host path is embedded.
#
# Requirements:
#   - Contiki-NG release/v4.9 (c20b12cd2db707d6c07918f8b26cfeb52cf298ca) with
#     the arch/cpu/arm/CMSIS submodule (CONTIKI_NG, default
#     ~/ext/contiki-ng-src/contiki-ng). Cloned for you if absent.
#   - arm-none-eabi-gcc on PATH (tested with 13.2.1).
#
# WERROR=0: GCC 13 reports -Warray-bounds on Contiki-NG's ROM_UTIL_API casts
# (a struct pointer at the fixed ROM address 0x48), and Contiki-NG builds with
# -Werror. The warning does not change code generation; no source is patched.
#
# cfs-coffee also builds os/services/unit-test, whose PRIu64/PRId64 the
# arm-none-eabi newlib <inttypes.h> leaves undefined under -std=c99. They are
# supplied through the environment CFLAGS, which Contiki-NG's `CFLAGS +=`
# appends to (unit-test is linked into neither ELF).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURES="$HERE/../../tests/fixtures"
CONTIKI_NG="${CONTIKI_NG:-$HOME/ext/contiki-ng-src/contiki-ng}"
REV="c20b12cd2db707d6c07918f8b26cfeb52cf298ca"

if [ ! -d "$CONTIKI_NG/.git" ]; then
  git clone --branch release/v4.9 https://github.com/contiki-ng/contiki-ng.git "$CONTIKI_NG"
fi
git -C "$CONTIKI_NG" checkout --quiet "$REV"
git -C "$CONTIKI_NG" submodule update --init --depth 1 arch/cpu/arm/CMSIS

cd "$CONTIKI_NG/examples/hello-world"
make TARGET=cc2538dk clean >/dev/null
make -j"$(nproc)" TARGET=cc2538dk WERROR=0
arm-none-eabi-size build/cc2538dk/hello-world.elf
cp build/cc2538dk/hello-world.elf "$FIXTURES/cc2538dk-contiki-hello.elf"

cd "$CONTIKI_NG/examples/storage/cfs-coffee"
make TARGET=cc2538dk clean >/dev/null
CFLAGS='-DPRIu64="\"llu\"" -DPRId64="\"lld\""' \
  make -j"$(nproc)" TARGET=cc2538dk WERROR=0
arm-none-eabi-strip --strip-debug -o "$FIXTURES/cc2538dk-contiki-test-cfs.elf" \
  build/cc2538dk/test-cfs.elf
arm-none-eabi-strip --strip-debug -o "$FIXTURES/cc2538dk-contiki-coffee.elf" \
  build/cc2538dk/example-coffee.elf

# Out-of-tree app: build a copy under examples/ so CONTIKI stays relative.
app="$CONTIKI_NG/examples/labwired-flash-test"
rm -rf "$app"
cp -r "$HERE/flash-test" "$app"
cd "$app"
make -j"$(nproc)" TARGET=cc2538dk WERROR=0 CONTIKI=../..
arm-none-eabi-strip --strip-debug -o "$FIXTURES/cc2538dk-contiki-flash-test.elf" \
  build/cc2538dk/flash-test.elf
cd "$CONTIKI_NG"
rm -rf "$app"

cd "$FIXTURES"
sha256sum cc2538dk-contiki-hello.elf cc2538dk-contiki-test-cfs.elf \
  cc2538dk-contiki-coffee.elf cc2538dk-contiki-flash-test.elf
