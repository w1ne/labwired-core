# Arduino × LabWired board matrix

_Generated 2026-11-09 06:10:39 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `stm32h723zg` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |

## Summary

- Cells: **9**
- **pass: 8** · **skip: 1** · **fail: 0**

### Status breakdown

- `pass`: 8
- `skipped`: 1

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `stm32h723zg` × `L8_can` → **skipped**
No H7 FDCAN model: the engine's fdcan model is the H5 fixed-layout FDCAN, while the H7's is the full M_CAN with a configurable message RAM at 0x4000AC00 (RM0468), so the H723 profile does not declare FDCAN
