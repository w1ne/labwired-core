#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
build_dir="$repo_root/examples/lolin-c3-mini-adc/firmware/.pio/build/lolin_c3_mini"

pio run \
  --project-dir "$repo_root/examples/lolin-c3-mini-adc/firmware" \
  --environment lolin_c3_mini
pio pkg exec -p tool-esptool -- esptool --chip esp32c3 merge-bin \
  --flash-mode dio --flash-freq 80m --flash-size 4MB \
  --output "$build_dir/merged.bin" \
  0x0 "$build_dir/bootloader.bin" \
  0x8000 "$build_dir/partitions.bin" \
  0x10000 "$build_dir/firmware.bin"
