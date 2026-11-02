#!/usr/bin/env bash
# Build stock upstream Apache NuttX for the Microchip Curiosity PIC32MZ EF 2.0
# (DM320209, PIC32MZ2048EFM144) using the out-of-tree board in ./board.
#
#   NUTTX=~/src/nuttx-m NUTTX_APPS=~/src/nuttx-apps-m \
#   CROSS=~/tools/mips-zephyr-elf-dl/mips-zephyr-elf/bin/mips-zephyr-elf- \
#   ./build.sh nsh|ostest
#
# NuttX and nuttx-apps are unmodified upstream master commits:
#   apache/nuttx      7956039dfddbe4b7d1ab2abfdac00f54606e63b0 (2026-10-08)
#   apache/nuttx-apps 1f8cac8b87a925403340947582b026cf2a117423 (2026-10-08)
# 7956039dfd ("arch/mips/pic32mz: record the running task on interrupt
# exit") is the first commit whose PIC32MZ port survives a context switch:
# before it, pic32mz_decodeirq() never updated g_running_tasks[], so every
# interrupt overwrote the Idle task's saved registers. No release tag
# (13.1.0 and earlier) contains it. The toolchain is the Zephyr SDK 0.17.0
# mips-zephyr-elf GCC 12.2 (any bare-metal little-endian MIPS32r2 GCC works).
# kconfiglib must be importable (`pip install kconfiglib`).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
cfg="${1:-nsh}"
NUTTX="${NUTTX:-$HOME/src/nuttx-m}"
NUTTX_APPS="${NUTTX_APPS:-$HOME/src/nuttx-apps-m}"
CROSS="${CROSS:-$HOME/tools/mips-zephyr-elf-dl/mips-zephyr-elf/bin/mips-zephyr-elf-}"
export PATH="${KCONFIG_VENV:-$HOME/tools/nuttx-venv}/bin:$PATH"
# kconfig-frontends is optional: fall back to the kconfig-tweak shim here.
command -v kconfig-tweak >/dev/null || export PATH="$PATH:$here"

# NuttX expects the apps tree at ../apps and the custom board at the path in
# the defconfig (../labwired-board, relative to the NuttX top directory).
top="$(dirname "$NUTTX")"
ln -sfn "$NUTTX_APPS" "$top/apps"
ln -sfn "$here/board" "$top/labwired-board"

# NuttX stamps __DATE__/__TIME__ into uname(); pin them to the NuttX
# commit's time so a rebuild is byte-identical.
export SOURCE_DATE_EPOCH=1791462501

cd "$NUTTX"
make distclean >/dev/null 2>&1 || true
./tools/configure.sh -l "$here/board/configs/$cfg"
make olddefconfig >/dev/null
# Serial: a parallel build adds the apps' objects to libapps.a in whatever
# order they finish, which moves code around between otherwise identical
# builds.
make -j1 CROSSDEV="$CROSS"

out="$repo/tests/fixtures/curiosity-pic32mz-ef-nuttx-$cfg.elf"
"${CROSS}strip" --strip-debug -o "$out" nuttx
chmod 644 "$out"
echo "wrote $out"
sha256sum "$out"
