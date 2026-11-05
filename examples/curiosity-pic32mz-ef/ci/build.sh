#!/usr/bin/env bash
# Curiosity PIC32MZ EF 2.0: nothing to build in CI. The NuttX fixtures are
# committed (tests/fixtures/curiosity-pic32mz-ef-nuttx-*.elf) and rebuilt
# byte-identically by crates/firmware-curiosity-pic32mz-nuttx/build.sh on a
# host with a bare-metal little-endian MIPS32r2 GCC.
set -euo pipefail
