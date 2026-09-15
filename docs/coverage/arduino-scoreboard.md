# Arduino × LabWired board matrix

_Generated 2026-08-12 20:11:39 +0200 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `esp32` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `esp32c3` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `esp32s3` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `nrf52832` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `nrf52840` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `rp2040` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32f103` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `stm32f401` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32f407` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `stm32g474re` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32h563` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `stm32l073` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32l476` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |  |
| `stm32wb55` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |
| `stm32wba52` | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | ✅ | ✅ | ⏭️ | L5_adc:skipped; L8_can:skipped |
| `atmega328p` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | L8_can:skipped |

## Summary

- Cells: **144**
- **pass: 133** · **skip: 11** · **fail: 0**

### Status breakdown

- `pass`: 133
- `skipped`: 11

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `esp32c3` × `L8_can` → **skipped**
TWAI0 declarative-only (no self-RX model for matrix L8)

### `nrf52832` × `L8_can` → **skipped**
No on-chip CAN model

### `nrf52840` × `L8_can` → **skipped**
No on-chip CAN model

### `rp2040` × `L8_can` → **skipped**
No on-chip CAN model

### `stm32f401` × `L8_can` → **skipped**
No bxCAN/FDCAN in chip yaml

### `stm32g474re` × `L8_can` → **skipped**
No FDCAN in chip yaml for matrix L8

### `stm32l073` × `L8_can` → **skipped**
No CAN peripheral on L0 series

### `stm32wb55` × `L8_can` → **skipped**
No CAN model in matrix path

### `stm32wba52` × `L5_adc` → **skipped**
No ADC model in stm32wba52 chip yaml yet

### `stm32wba52` × `L8_can` → **skipped**
No CAN model in matrix path

### `atmega328p` × `L8_can` → **skipped**
ATmega328P has no CAN controller

