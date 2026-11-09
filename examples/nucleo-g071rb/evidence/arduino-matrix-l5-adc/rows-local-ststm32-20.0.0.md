# Arduino × LabWired board matrix

_Generated 2026-11-09 12:24:45 +0000 by `validation/arduino-matrix/run_matrix.py`._

Legend: ✅ pass · 🔧 compile/build fail · 📦 toolchain missing · 🔴 boot/sim fail · 🟠 oracle miss · 🟣 unmodeled · ⏱️ timeout

| chip | L0_serial_boot | L1_serial_loop | L2_blink_serial | L3_i2c_sensor | L4_spi_sensor | L5_adc | L6_pwm | L7_timer | L8_can | notes |
|------|------|------|------|------|------|------|------|------|------|-------|
| `stm32g071rb` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ⏭️ | L4_spi_sensor:oracle_fail; L8_can:skipped |
| `stm32c031c6` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⏭️ | ✅ | ⏭️ | L6_pwm:skipped; L8_can:skipped |
| `stm32g0b1re` | ✅ | ✅ | ✅ | ✅ | 🟠 | ✅ | ✅ | ✅ | ✅ | L4_spi_sensor:oracle_fail |

## Summary

- Cells: **27**
- **pass: 22** · **skip: 3** · **fail: 2**

### Status breakdown

- `pass`: 22
- `skipped`: 3
- `oracle_fail`: 2

_Skips are explicit `boards.yaml` gaps (honest non-runs). Do not report pass+skip as a single “green” total._

## Non-pass detail (fails + skips)

### `stm32g071rb` × `L4_spi_sensor` → **oracle_fail**
```
[2m2026-11-09T12:23:02.810098Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../../configs/chips/stm32g071.yaml
[2m2026-11-09T12:23:02.874801Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x8004715
labwired-cli test: idle_ff=on (event_scheduler=on)
labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
[2m2026-11-09T12:23:10.932773Z[0m [31mERROR[0m [2mlabwired_cli::execute[0m[2m:[0m Assertion failed: UartContains(UartContainsAssertion { uart_contains: "LW_L4_OK" }) (captured len=12)
FAIL  0/1 checks · test · 20000000 steps · 8.05s

```
UART tail:
```
LW_L4_BOOT

```

### `stm32g071rb` × `L8_can` → **skipped**
STM32G071 has no CAN peripheral

### `stm32c031c6` × `L6_pwm` → **skipped**
Lane predates main #1734 (STM32 timer AF pad routing); the C0 pad table ships in the main PR — on the lane PA5 shows 0 edges

### `stm32c031c6` × `L8_can` → **skipped**
STM32C031C6 has no CAN peripheral

### `stm32g0b1re` × `L4_spi_sensor` → **oracle_fail**
```
[2m2026-11-09T12:24:20.445355Z[0m [32m INFO[0m [2mlabwired_core::system::builder[0m[2m:[0m Loading chip descriptor: ../../../configs/chips/stm32g0b1re.yaml
[2m2026-11-09T12:24:20.532652Z[0m [32m INFO[0m [2mlabwired_loader[0m[2m:[0m ELF Entry Point: 0x8004ae1
labwired-cli test: idle_ff=on (event_scheduler=on)
labwired-cli test: hosted fast path (exact windows up to 1024 cycles)
[2m2026-11-09T12:24:28.137178Z[0m [31mERROR[0m [2mlabwired_cli::execute[0m[2m:[0m Assertion failed: UartContains(UartContainsAssertion { uart_contains: "LW_L4_OK" }) (captured len=12)
FAIL  0/1 checks · test · 20000000 steps · 7.60s

```
UART tail:
```
LW_L4_BOOT

```

