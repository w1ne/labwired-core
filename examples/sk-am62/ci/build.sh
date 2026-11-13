#!/usr/bin/env bash
# SK-AM62: nothing to build in CI. The Zephyr fixtures are committed
# (tests/fixtures/sk-am62-zephyr-*.elf) and rebuilt byte-identically by
# crates/firmware-sk-am62-zephyr/build.sh on a host with the Zephyr SDK
# AArch64 toolchain.
set -euo pipefail
