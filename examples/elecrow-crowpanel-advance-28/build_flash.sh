#!/usr/bin/env bash
# Build a flashable ESP32-S3 image for the Elecrow Arduino firmware probe.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$ROOT/.pio/build/crowpanel_advance_28"
PKGS="${PLATFORMIO_PACKAGES_DIR:-$HOME/.platformio/packages}"
"$PKGS/tool-esptool/esptool" --chip esp32s3 merge-bin \
  --output "$BUILD/flash.bin" --flash-mode dio --flash-freq 80m --flash-size 16MB \
  0x0 "$BUILD/bootloader.bin" \
  0x8000 "$BUILD/partitions.bin" \
  0xe000 "$PKGS/framework-arduinoespressif32/tools/partitions/boot_app0.bin" \
  0x10000 "$BUILD/firmware.bin"
