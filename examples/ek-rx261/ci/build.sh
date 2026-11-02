#!/usr/bin/env bash
# EK-RX261: nothing to build in CI. The Zephyr fixtures are committed
# (tests/fixtures/ek-rx261-zephyr-*.elf) and rebuilt byte-identically by
# crates/firmware-ek-rx261-zephyr/build.sh on a host with the Zephyr SDK RX
# toolchain.
set -euo pipefail
