# Arduino × LabWired board matrix

_Generated 2026-11-09 12:55:44 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `stm32g071rb` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32g0b1re` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |

## Summary

- Cells: **18**
- **pass: 17** · **skip: 1** · **fail: 0**

### Status breakdown

- `pass`: 17
- `skipped`: 1

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `stm32g071rb` × `L8_can` → **skipped**
STM32G071 has no CAN peripheral

