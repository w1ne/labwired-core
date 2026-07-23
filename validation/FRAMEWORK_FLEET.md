# Framework fleet — Arduino + Zephyr on every supported chip

**Goal:** for every LabWired-supported MCU, compile stock-style **Arduino** and
**Zephyr** firmware, execute under `labwired test`, and **attach real kits**
(I2C/SPI/UART/GPIO sensors, displays, radios) without hand-thunking.

This is the product bar beyond unit tests and UART-only CI fixtures.

## Depth levels

| Level | Arduino | Zephyr | Proves |
|-------|---------|--------|--------|
| **L0** | `L0_serial_boot` | `L0_hello` | Core/kernel boot + console UART |
| **L1** | `L1_serial_loop` | `L1_sleep` | Loop / `k_msleep` scheduling |
| **L2** | `L2_blink_serial` | `L2_blink` | GPIO / LED + serial (+ optional logic edges) |
| **L3** | `L3_i2c_sensor` | `L3_i2c_sensor` | **I2C kit** on Wire / Zephyr `i2c` (INA219 @ 0x40) |
| **L4** | *(planned)* | *(planned)* | SPI / multi-device kits (MAX31855, display, …) |

## Chip coverage (record of intent)

| Chip | Arduino matrix | Zephyr matrix | L3 I2C kit | Notes |
|------|----------------|---------------|------------|-------|
| esp32 | ✅ | ❌ no ESP Zephyr path yet | pilot skip (NACK) | FreeRTOS dual-core |
| esp32c3 | ✅ | ❌ | ✅ Arduino | RISC-V FreeRTOS |
| esp32s3 | ✅ | ❌ | pilot skip (NACK) | Dual-core |
| nrf52832 | ✅ | ✅ | ✅ Arduino; Zephyr system+i2c0 | TWIM overlay for Zephyr L3 |
| nrf52840 | ✅ | ✅ | ✅ both | Zephyr L3 green (TWIM overlay) |
| nrf5340 | ❌ | ✅ | ❌ no I2C model | Add Arduino when PIO path exists |
| nrf54l15 | ❌ | ❌ | — | Chip model present; frameworks TBD |
| rp2040 | ✅ | ✅ | ✅ Arduino; Zephyr L3 skip | DW transfer EAGAIN under Zephyr |
| stm32f103 | ✅ | ✅ | ✅ both | F1 Wire + Zephyr poll green |
| stm32f401 | ✅ | ✅ | ✅ both | |
| stm32f407 | ✅ | ❌ | ✅ Arduino | Add Zephyr `nucleo_f407zg` if west board ok |
| stm32g474re | ✅ | ✅ | L3 skip L4-class | Wire.begin / L4 I2C hang WIP |
| stm32h563 | ✅ | ✅ | L3 skip L4-class | |
| stm32h735 | ❌ | ❌ | — | Chip model; framework path TBD |
| stm32l073 | ✅ | ✅ | L3 skip L4-class | |
| stm32l476 | ✅ | ✅ | L3 skip L4-class | |
| stm32wb55 | ✅ | ✅ | L3 skip L4-class | |
| stm32wba52 | ✅ | ✅ | L3 skip L4-class | Custom PIO board JSON |
| mkw41z4 | ❌ | ✅ | Zephyr system ready | Zephyr only today |

Legend: ✅ in `validation/*-matrix/boards.yaml` · ❌ not yet · — blocked

## How peripherals attach

1. **System YAML** (`validation/{arduino,zephyr}-matrix/systems/<board>.yaml`):
   `external_devices` lists kits (`type: ina219`, `connection: i2c0|i2c1|twi1`).
2. **Kit registry / I2C factory** (`crates/core/src/peripherals/`) implements the device.
3. **Firmware** uses stock Arduino `Wire` / Zephyr `i2c` drivers — no LabWired SDK.
4. **Oracle**: UART marker `LW_L3_OK` / `LW_Z3_OK` after successful I2C probe.

L0–L2 systems **keep** the INA219 attached; unused devices must not break UART
oracles (RP2040 attach path requires real I2C downcast).

## Commands

```bash
# Arduino full depth (needs PlatformIO platforms)
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py
python3 validation/arduino-matrix/run_matrix.py --sketches L3_i2c_sensor
python3 validation/arduino-matrix/run_matrix.py --boards stm32f103,esp32c3 --sketches L0_serial_boot,L3_i2c_sensor

# Zephyr (needs west + ZEPHYRPROJECT, default ~/zephyrproject)
python3 validation/zephyr-matrix/run_matrix.py --zephyrproject ~/zephyrproject
python3 validation/zephyr-matrix/run_matrix.py --boards stm32f103,nrf52840 --levels L0_hello,L3_i2c_sensor

# Coverage table
python3 validation/framework_fleet_report.py
```

## CI

| Job | Scope |
|-----|--------|
| `arduino-matrix-gate` | All Arduino boards × L0+L2+L3 (skipped cells ok) |
| Onboarding / coverage-matrix | Bare-metal UART fixtures (not framework stock) |
| Zephyr matrix | **Not yet in CI** (needs west image) — run locally |

## Roadmap

1. **Arduino L3 green** on F1 / nRF / RP2040 / C3 (landed).
2. **Zephyr L3** sample + systems (this change); prove F1/nRF/RP first.
3. Fix STM32 L4-class `Wire.begin` / Zephyr I2C master hang (RCC timing).
4. Fix ESP classic/S3 Arduino Wire NACK path.
5. SPI L4 kits (MAX31855, display) on Arduino + Zephyr.
6. ESP Zephyr when survival path exists; Zephyr matrix in CI west container.
