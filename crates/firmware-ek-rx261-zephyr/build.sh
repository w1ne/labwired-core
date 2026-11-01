#!/usr/bin/env bash
# Rebuild the EK-RX261 (Renesas R5F52618BGFP, RX261) Zephyr fixtures:
#   tests/fixtures/ek-rx261-zephyr-hello.elf         — samples/hello_world
#     (examples/ek-rx261/zephyr-hello.yaml)
#   tests/fixtures/ek-rx261-zephyr-sync.elf          — samples/synchronization
#     (examples/ek-rx261/zephyr-sync.yaml)
#   tests/fixtures/ek-rx261-zephyr-kernel-common.elf — tests/kernel/common
#     (examples/ek-rx261/zephyr-kernel-common.yaml)
#
# UNMODIFIED upstream Zephyr v4.3.0 (commit 3568e1b6d5cd) for board target
# `ek_rx261`, the first Zephyr release with RX261 support, with hal_renesas
# at the revision v4.3.0's west.yml pins (a279c14e196c). The ELFs have their
# `.debug*` sections dropped (symbols kept for fault reports) so build-host
# paths in DWARF do not leak in; that is also what makes a rebuild
# byte-identical. `strip_debug.py` does that rather than binutils `strip`,
# which corrupts the segment holding the RX fixed vector table (see there).
#
# Requirements:
#   - Zephyr v4.3.0 (ZEPHYR_BASE, default ~/zephyrproject-v4.3/zephyr) and
#     hal_renesas (HAL_RENESAS, default ~/zephyrproject-v4.3/modules/hal/renesas).
#     A shallow clone of each is enough; no west workspace is needed:
#       git clone --depth 1 --branch v4.3.0 https://github.com/zephyrproject-rtos/zephyr
#       git -C hal_renesas fetch --depth 1 origin a279c14e196cb0904593035888a334dcd2c720bd
#   - Zephyr's Python requirements (cmake, ninja, pyelftools, ...).
#   - The Zephyr SDK 1.0.1 RX toolchain (`rx-zephyr-elf`, GCC 14.3.0), used
#     through the `cross-compile` variant; only that one archive is needed:
#       gh release download v1.0.1 -R zephyrproject-rtos/sdk-ng \
#         -p 'toolchain_gnu_linux-x86_64_rx-zephyr-elf.tar.xz'
#     then CROSS_COMPILE=<dir>/rx-zephyr-elf/bin/rx-zephyr-elf-
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ZEPHYR_BASE="${ZEPHYR_BASE:-$HOME/zephyrproject-v4.3/zephyr}"
HAL_RENESAS="${HAL_RENESAS:-$HOME/zephyrproject-v4.3/modules/hal/renesas}"
BOARD="ek_rx261"
FIXTURES="$HERE/../../tests/fixtures"

export ZEPHYR_BASE
export ZEPHYR_TOOLCHAIN_VARIANT=cross-compile
export CROSS_COMPILE="${CROSS_COMPILE:-$HOME/tools/rx-zephyr-elf-dl/rx-zephyr-elf/bin/rx-zephyr-elf-}"

echo "Zephyr:    $ZEPHYR_BASE ($(git -C "$ZEPHYR_BASE" describe --tags 2>/dev/null || echo '?'))"
echo "HAL:       $HAL_RENESAS ($(git -C "$HAL_RENESAS" rev-parse --short=12 HEAD 2>/dev/null || echo '?'))"
echo "Board:     $BOARD"
echo "Toolchain: ${CROSS_COMPILE}gcc $("${CROSS_COMPILE}gcc" -dumpfullversion)"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi
}

build_one() { # <sample-dir> <fixture-name>
  local build; build="$(mktemp -d)"
  cmake -GNinja -S "$ZEPHYR_BASE/$1" -B "$build" -DBOARD="$BOARD" \
    "-DZEPHYR_MODULES=$HAL_RENESAS" >/dev/null
  ninja -C "$build" >/dev/null
  python3 -I "$HERE/strip_debug.py" "$build/zephyr/zephyr.elf" "$FIXTURES/$2"
  chmod 0644 "$FIXTURES/$2"
  rm -rf "$build"
  "${CROSS_COMPILE}size" "$FIXTURES/$2"
  sha256 "$FIXTURES/$2"
}

build_one "samples/hello_world"     "ek-rx261-zephyr-hello.elf"
build_one "samples/synchronization" "ek-rx261-zephyr-sync.elf"
build_one "tests/kernel/common"     "ek-rx261-zephyr-kernel-common.elf"
