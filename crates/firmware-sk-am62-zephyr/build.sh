#!/usr/bin/env bash
# Rebuild the SK-AM62 (TI AM6234, Cortex-A53) Zephyr fixtures:
#   tests/fixtures/sk-am62-zephyr-hello.elf         — samples/hello_world
#     (examples/sk-am62/zephyr-hello.yaml)
#   tests/fixtures/sk-am62-zephyr-sync.elf          — samples/synchronization
#     (examples/sk-am62/zephyr-sync.yaml)
#   tests/fixtures/sk-am62-zephyr-kernel-common.elf — tests/kernel/common
#     (examples/sk-am62/zephyr-kernel-common.yaml)
#
# UNMODIFIED upstream Zephyr v4.3.0 (commit 3568e1b6d5cd) for board target
# `sk_am62/am6234/a53`, with picolibc at the revision v4.3.0's west.yml pins
# (ca8b6ebba522). The ELFs have their `.debug*` sections dropped (symbols
# kept for fault reports) so build-host paths in DWARF do not leak in; that
# is also what makes a rebuild byte-identical.
#
# Requirements:
#   - Zephyr v4.3.0 (ZEPHYR_BASE, default ~/zephyrproject-v4.3/zephyr) and
#     picolibc (PICOLIBC, default ~/zephyrproject-v4.3/modules/lib/picolibc).
#     A shallow clone of each is enough; no west workspace is needed:
#       git clone --depth 1 --branch v4.3.0 https://github.com/zephyrproject-rtos/zephyr
#       git -C picolibc fetch --depth 1 origin ca8b6ebba5226a75545e57a140443168a26ba664
#   - Zephyr's Python requirements (cmake, ninja, pyelftools, ...) and gperf
#     (tests/kernel/common turns on CONFIG_USERSPACE on arm64).
#   - The Zephyr SDK 1.0.1 AArch64 toolchain (`aarch64-zephyr-elf`, GCC
#     14.3.0), used through the `cross-compile` variant; only that one archive
#     is needed:
#       gh release download v1.0.1 -R zephyrproject-rtos/sdk-ng \
#         -p 'toolchain_gnu_linux-x86_64_aarch64-zephyr-elf.tar.xz'
#     then CROSS_COMPILE=<dir>/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYR_BASE="${ZEPHYR_BASE:-$HOME/zephyrproject-v4.3/zephyr}"
PICOLIBC="${PICOLIBC:-$HOME/zephyrproject-v4.3/modules/lib/picolibc}"
BOARD="sk_am62/am6234/a53"
FIXTURES="$HERE/../../tests/fixtures"

export ZEPHYR_BASE
export ZEPHYR_TOOLCHAIN_VARIANT=cross-compile
export CROSS_COMPILE="${CROSS_COMPILE:-$HOME/tools/aarch64-zephyr-elf-dl/aarch64-zephyr-elf/bin/aarch64-zephyr-elf-}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "picolibc:  $PICOLIBC ($(git -C "$PICOLIBC" rev-parse --short=12 HEAD 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: ${CROSS_COMPILE}gcc $("${CROSS_COMPILE}gcc" -dumpfullversion)"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi
}

build_one() { # <sample-dir> <fixture-name>
  local build; build="$(mktemp -d)"
  cmake -GNinja -S "$ZEPHYR_BASE/$1" -B "$build" -DBOARD="$BOARD" \
    "-DZEPHYR_MODULES=$PICOLIBC" >/dev/null
  ninja -C "$build" >/dev/null
  "${CROSS_COMPILE}strip" --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 0644 "$FIXTURES/$2"
  rm -rf "$build"
  "${CROSS_COMPILE}size" "$FIXTURES/$2"
  sha256 "$FIXTURES/$2"
}

build_one "samples/hello_world"     "sk-am62-zephyr-hello.elf"
build_one "samples/synchronization" "sk-am62-zephyr-sync.elf"
build_one "tests/kernel/common"     "sk-am62-zephyr-kernel-common.elf"
