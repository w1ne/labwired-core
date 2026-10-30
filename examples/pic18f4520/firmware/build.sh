#!/usr/bin/env bash
# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# Rebuild the PIC18F4520 smoke firmware.
#
# Toolchain (pinned; the committed image was built with exactly these):
#   - SDCC 4.4.0 upstream amd64 release (sdcc-4.4.0-amd64-unknown-linux2.5,
#     sha256 7cb4c96bd774f95e8032746e6808b59f3a8e2b3e7b4c72ac566bb67beeacff80).
#     Distribution SDCC packages often omit the pic16 port.
#   - gputils 1.4.0 (gpasm/gplink, Ubuntu `gputils` package).
#
# SDCC=/path/to/sdcc ./build.sh
set -euo pipefail
cd "$(dirname "$0")"
SDCC="${SDCC:-sdcc}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp sdcc-uart-smoke.c "$work/"
(cd "$work" && "$SDCC" -mpic16 -p18f4520 --use-non-free --opt-code-size sdcc-uart-smoke.c)
cp "$work/sdcc-uart-smoke.hex" pic18f4520-sdcc-uart-smoke.hex
python3 hex2elf.py pic18f4520-sdcc-uart-smoke.hex pic18f4520-sdcc-uart-smoke.elf
sha256sum pic18f4520-sdcc-uart-smoke.hex pic18f4520-sdcc-uart-smoke.elf
