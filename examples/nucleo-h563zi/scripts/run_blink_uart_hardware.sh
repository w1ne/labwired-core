#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXAMPLE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BOARD_FW_DIR="$EXAMPLE_DIR/board_firmware"

UART_PORT="${UART_PORT:-}"
UART_TIMEOUT="${UART_TIMEOUT:-8}"
OPENOCD_INTERFACE="${OPENOCD_INTERFACE:-interface/stlink-dap.cfg}"
OPENOCD_TRANSPORT="${OPENOCD_TRANSPORT:-dapdirect_swd}"
OPENOCD_TARGET="${OPENOCD_TARGET:-target/stm32h5x.cfg}"
DO_FLASH=1
CAT_PID=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --port)
      UART_PORT="${2:-}"
      shift 2
      ;;
    --timeout)
      UART_TIMEOUT="${2:-}"
      shift 2
      ;;
    --no-flash)
      DO_FLASH=0
      shift
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      echo "usage: $0 [--port /dev/ttyACM0] [--timeout 8] [--no-flash]" >&2
      exit 2
      ;;
  esac
done

if ! command -v openocd >/dev/null 2>&1; then
  echo "error: openocd is required" >&2
  exit 2
fi

if ! command -v arm-none-eabi-gcc >/dev/null 2>&1; then
  echo "error: arm-none-eabi-gcc is required" >&2
  exit 2
fi

if [[ -z "$UART_PORT" ]]; then
  if compgen -G "/dev/serial/by-id/*STLINK*" >/dev/null 2>&1; then
    UART_PORT="$(ls /dev/serial/by-id/*STLINK* | head -n 1)"
  elif [[ -e /dev/ttyACM0 ]]; then
    UART_PORT="/dev/ttyACM0"
  fi
fi

if [[ -z "$UART_PORT" ]]; then
  echo "error: UART port not found. pass --port /dev/ttyACM*" >&2
  exit 2
fi

echo "Using UART port: $UART_PORT"

echo
echo "==> Building real-board firmware"
make -C "$BOARD_FW_DIR" clean
make -C "$BOARD_FW_DIR"

ELF="$BOARD_FW_DIR/build/h563_blink_uart.elf"
if [[ ! -f "$ELF" ]]; then
  echo "error: firmware ELF missing at $ELF" >&2
  exit 2
fi

LOG_FILE="$(mktemp /tmp/h563_blink_uart.XXXXXX.log)"
echo "UART capture log: $LOG_FILE"

stty -F "$UART_PORT" 115200 cs8 -cstopb -parenb -ixon -ixoff -crtscts -echo raw

cleanup() {
  if [[ -n "$CAT_PID" ]]; then
    kill "$CAT_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo
echo "==> Capturing UART for ${UART_TIMEOUT}s"
timeout "$UART_TIMEOUT" cat "$UART_PORT" >"$LOG_FILE" 2>/dev/null &
CAT_PID=$!

sleep 0.2

if [[ "$DO_FLASH" == "1" ]]; then
  echo
  echo "==> Flashing firmware with OpenOCD"
  openocd -f "$OPENOCD_INTERFACE" -c "transport select $OPENOCD_TRANSPORT" -f "$OPENOCD_TARGET" \
    -c "program $ELF verify reset exit"
else
  echo
  echo "==> Skipping flash (--no-flash). Waiting for running firmware output."
fi

wait "$CAT_PID" || true
CAT_PID=""
trap - EXIT

echo
echo "==> UART output"
cat "$LOG_FILE"

if ! grep -q "H563-BLINK-UART" "$LOG_FILE"; then
  echo "error: UART banner not found" >&2
  exit 1
fi

if ! grep -q "BLINK" "$LOG_FILE"; then
  echo "error: BLINK lines not found" >&2
  exit 1
fi

if ! grep -q "PB0=1" "$LOG_FILE" || ! grep -q "PB0=0" "$LOG_FILE"; then
  echo "error: LED toggle evidence for PB0 not found" >&2
  exit 1
fi

if ! grep -q "PF4=1" "$LOG_FILE" || ! grep -q "PF4=0" "$LOG_FILE"; then
  echo "error: LED toggle evidence for PF4 not found" >&2
  exit 1
fi

if ! grep -q "PG4=1" "$LOG_FILE" || ! grep -q "PG4=0" "$LOG_FILE"; then
  echo "error: LED toggle evidence for PG4 not found" >&2
  exit 1
fi

echo
echo "Hardware blink+UART check passed."
