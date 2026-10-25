#!/bin/sh
# Rebuild loop_lab.elf from loop_lab.rs.
#
# The RP2040 is a Cortex-M0+ (thumbv6m), so the firmware builds with the
# installed Rust target and its bundled lld — no arm-none-eabi toolchain
# needed. The vector table (SP + reset) is the first thing in `.text`, which
# is placed at the flash base 0x10000000 where the RP2040 boots.
set -e
cd "$(dirname "$0")"
rustc --edition 2021 --target thumbv6m-none-eabi -O -C panic=abort \
  -C link-arg=--build-id=none \
  -C link-arg=-Ttext=0x10000000 \
  -C link-arg=--entry=reset \
  -o loop_lab.elf loop_lab.rs
