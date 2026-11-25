#!/usr/bin/env bash
# Rebuild the ATmega328PB Arduino fixtures with PlatformIO atmelavr
# (MiniCore variant atmega328pb) and copy the ELFs beside this script.
set -euo pipefail
cd "$(dirname "$0")"
HERE="$PWD"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/src"
cp main.cpp "$WORK/src/main.cpp"
cp platformio.ini "$WORK/platformio.ini"
(cd "$WORK" && pio run)
cp "$WORK/.pio/build/blink/firmware.elf" "$HERE/../atmega328pb-arduino-blink.elf"
cp "$WORK/.pio/build/control/firmware.elf" "$HERE/../atmega328pb-arduino-control.elf"
