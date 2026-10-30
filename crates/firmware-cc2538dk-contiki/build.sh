#!/usr/bin/env bash
# Rebuild tests/fixtures/cc2538dk-contiki-hello.elf: the UNMODIFIED upstream
# Contiki-NG examples/hello-world built for TARGET=cc2538dk (full default
# netstack: RPL Lite, 6LoWPAN, CSMA on the cc2538 RF driver). The committed ELF
# is the source of truth; this script reproduces it.
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
sha256sum "$FIXTURES/cc2538dk-contiki-hello.elf"
