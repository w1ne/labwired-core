#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

"$SCRIPT_DIR/run_blink_uart_emulator.sh"
"$SCRIPT_DIR/run_blink_uart_hardware.sh" "$@"
