#!/usr/bin/env bash
# Capture reset-state register windows from a live ESP32-S3 over the built-in
# USB-JTAG, via openocd-esp32. Mirror of esp32c3_capture_estate.sh.
#
# Reads the control/config register span of the peripheral blocks the sim wires
# in configs/chips/esp32s3.yaml. State is reset-halt (WARM, post-ROM); clock-gated
# blocks read back 0x0 / 0xffffffff or raise a load fault — that is itself signal.
#
# When more than one Espressif USB-JTAG board is attached, pass the S3's serial
# (its MAC, from `ioreg`/`system_profiler`) so openocd selects the right adapter:
#   ESP32S3_SERIAL=9C:13:9E:F4:40:C0 esp32s3_capture_estate.sh <out_dir>
set -uo pipefail

OOCD_HOME="${OOCD_HOME:-/private/tmp/openocd-esp32}"
OOCD="$OOCD_HOME/bin/openocd"
SCRIPTS="$OOCD_HOME/share/openocd/scripts"
OUT_DIR="${1:?usage: esp32s3_capture_estate.sh <out_dir>}"
SERIAL="${ESP32S3_SERIAL:-}"
mkdir -p "$OUT_DIR"
LOG="$OUT_DIR/openocd.log"

# block base word-count  (bases match configs/chips/esp32s3.yaml)
BLOCKS=(
  "uart0     0x60000000 32"
  "gpio      0x60004000 48"
  "i2c0      0x60013000 32"
  "rmt       0x60016000 48"
  "mcpwm0    0x6001e000 48"
  "timg0     0x6001f000 32"
  "systimer  0x60023000 32"
  "gdma      0x6003f000 64"
  "system    0x600c0000 32"
  "rtc_cntl  0x60008000 16"
)

CMDS=""
[ -n "$SERIAL" ] && CMDS+="adapter serial $SERIAL; "
CMDS+="adapter speed 4000; init; reset halt;"
for b in "${BLOCKS[@]}"; do
  set -- $b
  CMDS+=" echo {@@$1 $2}; echo [capture {mdw $2 $3}];"
done
CMDS+=" exit"

"$OOCD" -s "$SCRIPTS" -f "board/esp32s3-builtin.cfg" -c "$CMDS" > "$LOG" 2>&1
echo "openocd exit: $?"
grep -c "^@@" "$LOG" | sed 's/^/blocks captured: /'
echo "log: $LOG"
