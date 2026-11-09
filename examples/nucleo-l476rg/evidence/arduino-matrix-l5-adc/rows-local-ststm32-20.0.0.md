# Arduino × LabWired board matrix

_Generated 2026-11-09 14:08:42 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `stm32g474re` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ⏭️ | L4_spi_sensor:oracle_fail; L8_can:skipped |
| `stm32l073` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ⏭️ | L4_spi_sensor:oracle_fail; L8_can:skipped |
| `stm32l476` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ✅ | L4_spi_sensor:oracle_fail |
| `stm32wb55` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ⏭️ | L4_spi_sensor:oracle_fail; L8_can:skipped |

## Summary

- Cells: **36**
- **pass: 29** · **skip: 3** · **fail: 4**

### Status breakdown

- `pass`: 29
- `oracle_fail`: 4
- `skipped`: 3

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)
