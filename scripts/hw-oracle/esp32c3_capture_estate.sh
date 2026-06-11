#!/usr/bin/env bash
# Capture reset-state register windows for the newly-wired ESP32-C3 estate blocks
# from a live ESP32-C3 over the built-in USB-JTAG, via openocd-esp32.
#
# Emits one "@@<block> <base>" marker followed by mdw word dumps per block, into
# openocd.log. State is reset-halt (WARM post-ROM); clock-gated blocks may read
# back as 0x0/0xffffffff or raise a load fault — that is itself signal.
set -uo pipefail

OOCD_HOME="${OOCD_HOME:-/private/tmp/openocd-esp32}"
OOCD="$OOCD_HOME/bin/openocd"
SCRIPTS="$OOCD_HOME/share/openocd/scripts"
OUT_DIR="${1:?usage: esp32c3_capture_estate.sh <out_dir>}"
mkdir -p "$OUT_DIR"
LOG="$OUT_DIR/openocd.log"

# block base word-count  (count chosen to cover the control/config register span)
BLOCKS=(
  "spi1 0x60002000 64"
  "spi0 0x60003000 64"
  "gpio_sd 0x60004f00 16"
  "efuse 0x60008800 96"
  "uhci1 0x6000c000 32"
  "uhci0 0x60014000 32"
  "bb 0x6001d000 16"
  "twai0 0x6002b000 48"
  "i2s0 0x6002d000 96"
  "aes 0x6003a000 48"
  "sha 0x6003b000 48"
  "rsa 0x6003c000 64"
  "ds 0x6003d000 64"
  "hmac 0x6003e000 32"
  "dma 0x6003f000 96"
  "apb_saradc 0x60040000 48"
  "usb_device 0x60043000 64"
  "sensitive 0x600c1000 64"
  "extmem 0x600c4000 64"
  "xts_aes 0x600cc000 32"
  "assist_debug 0x600ce000 48"
)

CMDS="adapter speed 4000; riscv set_command_timeout_sec 10; init; reset halt;"
for b in "${BLOCKS[@]}"; do
  set -- $b
  CMDS+=" echo {@@$1 $2}; echo [capture {mdw $2 $3}];"
done
CMDS+=" exit"

"$OOCD" -s "$SCRIPTS" -f "board/esp32c3-builtin.cfg" \
  -c "$CMDS" > "$LOG" 2>&1
echo "openocd exit: $?"
grep -c "^@@" "$LOG" | sed 's/^/blocks captured: /'
echo "log: $LOG"
