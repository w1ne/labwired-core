# Arduino × LabWired board matrix

_Generated 2026-11-20 19:00:20 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `nucleo-h753zi` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |

## Summary

- Cells: **9**
- **pass: 8** · **skip: 1** · **fail: 0**

### Status breakdown

- `pass`: 8
- `skipped`: 1

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `nucleo-h753zi` × `L8_can` → **skipped**
H7 FDCAN message RAM is shared and placed by SIDFC/RXF0C/TXBC; the shared fdcan model uses the fixed H5/G4 SRAMCAN layout, so a loopback here would not be faithful

