#!/usr/bin/env bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# Rebuild the simctl self-test fixture.
#
# The ELF is checked in (tests/fixtures/) so the gate runs without an embedded
# toolchain, exactly like the other firmware fixtures in this repo. Run this
# after changing main.c or the generated header, and commit the result.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$root/tests/fixtures/simctl-selftest-thumbv7m.elf"

arm-none-eabi-gcc \
    -mcpu=cortex-m3 -mthumb \
    -Os -ffreestanding -nostdlib -fno-builtin \
    -Wall -Wextra -Werror \
    -I"$root/examples/common" \
    -T "$here/link.ld" \
    -o "$out" \
    "$here/main.c"

echo "wrote $out"
arm-none-eabi-size "$out" 2>/dev/null || true
