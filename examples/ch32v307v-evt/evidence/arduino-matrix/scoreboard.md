# Arduino × LabWired board matrix

_Generated 2026-11-08 03:54:32 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `ch32v307` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | ✅ | L7_timer:skipped |

## Summary

- Cells: **9**
- **pass: 8** · **skip: 1** · **fail: 0**

### Status breakdown

- `pass`: 8
- `skipped`: 1

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `ch32v307` × `L7_timer` → **skipped**
Upstream core bug, not a model gap: getCurrentMicros() (ch32/clock.c) computes (CMP+1-CNT), which assumes a down-counter, but hw_config.c starts the V4 STK with CTLR=0xF, MODE (bit 4) clear = count up, so micros() runs backwards inside each millisecond (same bug as the ch32v003 and ch32v203 rows)

