# Arduino × LabWired board matrix

_Generated 2026-11-09 07:28:45 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `stm32c031c6` | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | ✅ | ✅ | ⏭️ | L5_adc:skipped; L8_can:skipped |

## Summary

- Cells: **9**
- **pass: 7** · **skip: 2** · **fail: 0**

### Status breakdown

- `pass`: 7
- `skipped`: 2

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `stm32c031c6` × `L5_adc` → **skipped**
No C0 ADC (CHSELR) model: analogRead returns 0 on every pin; a pass would be vacuous

### `stm32c031c6` × `L8_can` → **skipped**
STM32C031C6 has no CAN peripheral

