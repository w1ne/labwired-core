#!/usr/bin/env bash
# Dual-universe smoke: MATRIX_SPEED + event-scheduler on Class-M boards only.
# ESP FreeRTOS labs are intentionally excluded (known event-scheduler gaps).
#
# Build once:
#   cargo build -p labwired-cli --release --features event-scheduler
#
# Run (from core/):
#   LABWIRED_MATRIX_SPEED=1 bash validation/arduino-matrix/scripts/matrix_speed_subset.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
export LABWIRED_MATRIX_SPEED="${LABWIRED_MATRIX_SPEED:-1}"
BOARDS="${BOARDS:-stm32f401,stm32f103,stm32l073,stm32wb55,nrf52840,rp2040}"
exec python3 validation/arduino-matrix/run_matrix.py \
  --sim-only \
  --boards "$BOARDS" \
  --sketches L0_serial_boot,L2_blink_serial \
  "$@"
