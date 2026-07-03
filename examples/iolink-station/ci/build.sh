#!/usr/bin/env bash
# Build the three STM32L476 IO-Link station firmwares (real stacks + CubeL4).
set -euo pipefail
: "${STM32CUBE_L4_DIR:?STM32CUBE_L4_DIR must be set (fetch-pack action)}"
ROOT="$(git rev-parse --show-toplevel)"

make -C "$ROOT/examples/iolink-dido/firmware"           STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/master-fw"        STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/master-fw-4port"  STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/device-fw-svc"    STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"
make -C "$ROOT/examples/iolink-station/master-fw-svc"     STM32CUBE_L4_DIR="$STM32CUBE_L4_DIR"

# Hard-fail if any ELF is missing, so a silent build miss can't slip the gate.
test -f "$ROOT/examples/iolink-dido/firmware/iolink_dido.elf"
test -f "$ROOT/examples/iolink-station/master-fw/master.elf"
test -f "$ROOT/examples/iolink-station/master-fw-4port/master.elf"
test -f "$ROOT/examples/iolink-station/device-fw-svc/device.elf"
test -f "$ROOT/examples/iolink-station/master-fw-svc/master.elf"
