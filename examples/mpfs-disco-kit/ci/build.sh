#!/usr/bin/env bash
# PolarFire SoC Discovery Kit: nothing to build in CI. The Zephyr fixtures are
# committed (tests/fixtures/mpfs-zephyr-*.elf) and rebuilt byte-identically by
# crates/firmware-mpfs-zephyr/build.sh on a host with a riscv64 GCC.
set -euo pipefail
