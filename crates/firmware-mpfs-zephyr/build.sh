#!/usr/bin/env bash
# Rebuild the PolarFire SoC Discovery Kit (MPFS095T) Zephyr fixtures:
#   tests/fixtures/mpfs-zephyr-hello.elf  — samples/hello_world
#     (examples/mpfs-disco-kit/zephyr-hello.yaml)
#   tests/fixtures/mpfs-zephyr-sync.elf   — samples/synchronization
#     (examples/mpfs-disco-kit/zephyr-sync.yaml)
#   tests/fixtures/mpfs-zephyr-kernel-common.elf — tests/kernel/common
#     (examples/mpfs-disco-kit/zephyr-kernel-common.yaml)
#
# UNMODIFIED upstream Zephyr v4.1.0 (commit 7823374e8721) for board target
# `mpfs_icicle/polarfire/u54`: the image boots on the first U54 hart
# (CONFIG_RV_BOOT_HART=1) and links into the DDR window at 0x8000_0000. Zephyr
# has no separate Discovery Kit board; both kits carry the same PolarFire SoC
# MSS (memory map, MMUARTs, CLINT, PLIC), and the u54 target uses only MSS
# blocks, so the image is the one that runs on an MPFS095T.
# The ELFs are debug-stripped (`--strip-debug`, symbols kept for fault
# reports) so build-host paths in DWARF do not leak in; that is also what
# makes a rebuild byte-identical.
#
# Requirements:
#   - A Zephyr v4.1.0 west workspace (ZEPHYRPROJECT, default
#     ~/zephyrproject-v4.1) with the picolibc module.
#   - A riscv64-unknown-elf GCC (the `cross-compile` toolchain variant; no
#     Zephyr SDK needed). Ubuntu 24.04 `gcc-riscv64-unknown-elf` 13.2.0 is what
#     the committed fixtures were built with. Without root:
#       apt-get download gcc-riscv64-unknown-elf binutils-riscv64-unknown-elf
#       dpkg -x <deb> <dir>; CROSS_COMPILE=<dir>/usr/bin/riscv64-unknown-elf-
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYRPROJECT="${ZEPHYRPROJECT:-$HOME/zephyrproject-v4.1}"
BOARD="mpfs_icicle/polarfire/u54"
FIXTURES="$HERE/../../tests/fixtures"

WEST="${WEST:-west}"

export ZEPHYR_BASE="$ZEPHYRPROJECT/zephyr"
export ZEPHYR_TOOLCHAIN_VARIANT=cross-compile
export CROSS_COMPILE="${CROSS_COMPILE:-riscv64-unknown-elf-}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: ${CROSS_COMPILE}gcc $("${CROSS_COMPILE}gcc" -dumpfullversion)"

# Pin the module list: west otherwise passes every module in the workspace,
# and each adds an absolute CONFIG_ZEPHYR_<MOD>_MODULE symbol to the ELF.
MODULES="$ZEPHYRPROJECT/modules/lib/picolibc"

build_one() { # <sample-dir> <fixture-name> [extra cmake args...]
  local build; build="$(mktemp -d)"
  "$WEST" build -p always -b "$BOARD" "$ZEPHYR_BASE/$1" -d "$build" \
    -- "-DZEPHYR_MODULES=$MODULES" "${@:3}"
  "${CROSS_COMPILE}strip" --strip-debug -o "$FIXTURES/$2" "$build/zephyr/zephyr.elf"
  chmod 0644 "$FIXTURES/$2"
  rm -rf "$build"
  "${CROSS_COMPILE}size" "$FIXTURES/$2"
  sha256sum "$FIXTURES/$2"
}

build_one "samples/hello_world"     "mpfs-zephyr-hello.elf"
build_one "samples/synchronization" "mpfs-zephyr-sync.elf"
build_one "tests/kernel/common"     "mpfs-zephyr-kernel-common.elf"
