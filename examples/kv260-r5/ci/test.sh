#!/usr/bin/env bash
# AMD Kria KV260 (XCK26 RPU, Cortex-R5F) CLI proof: stock Zephyr v4.1.0
# hello_world, synchronization and tests/kernel/common through a native CLI.
# The Cortex-R5F is the pinned tlib engine behind --features qemu-arm-system
# (GPL native build, x86_64 Linux GNU only), so this runs in the native ARM
# workflow (.github/workflows/core-qemu-arm.yml), not the default board CI.
# Busy-waits are real instructions (100 ms = ~53 M at 533 MHz), so the CLI is
# the native-system (release, panic=unwind) profile.
set -euo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
OUT=out/boards/kv260-r5
cargo build -q --locked --profile native-system -p labwired-cli --features qemu-arm-system,event-scheduler
target/native-system/labwired test \
  --script examples/kv260-r5/zephyr-hello.yaml \
  --output-dir "$OUT/zephyr-hello" \
  --no-uart-stdout
target/native-system/labwired test \
  --script examples/kv260-r5/zephyr-sync.yaml \
  --output-dir "$OUT/zephyr-sync" \
  --no-uart-stdout
target/native-system/labwired test \
  --script examples/kv260-r5/zephyr-kernel-common.yaml \
  --output-dir "$OUT/zephyr-kernel-common" \
  --no-uart-stdout
