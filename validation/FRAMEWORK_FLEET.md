# Framework fleet — Arduino + Zephyr on every supported chip

**Goal:** for every LabWired-supported MCU, compile stock-style **Arduino** and
**Zephyr** firmware, execute under `labwired test`, and **attach real kits**
(I2C/SPI/UART sensors, displays, radios) without hand-thunking.

This is the product bar beyond unit tests and UART-only CI fixtures.

## Depth levels

| Level | Arduino | Zephyr | Proves |
|-------|---------|--------|--------|
| **L0** | `L0_serial_boot` | `L0_hello` | Core/kernel boot + console UART |
| **L1** | `L1_serial_loop` | `L1_sleep` | Loop / `k_msleep` scheduling |
| **L2** | `L2_blink_serial` | `L2_blink` | GPIO / LED + serial (+ optional logic edges) |
| **L3** | `L3_i2c_sensor` | *(planned)* | **Peripheral kit** on Wire/I2C (INA219 @ 0x40) |

## Chip coverage (record of intent)

| Chip | Arduino matrix | Zephyr matrix | L3 I2C kit | Notes |
|------|----------------|---------------|------------|-------|
| esp32 | ✅ | ❌ no ESP Zephyr path yet | ✅ ina219@i2c0 | FreeRTOS dual-core |
| esp32c3 | ✅ | ❌ | ✅ | RISC-V FreeRTOS |
| esp32s3 | ✅ | ❌ | ✅ | Dual-core |
| nrf52832 | ✅ | ✅ | ✅ | |
| nrf52840 | ✅ | ✅ | ✅ | |
| nrf5340 | ❌ | ✅ | — | Add Arduino when PIO path exists |
| nrf54l15 | ❌ | ❌ | — | Chip model present; frameworks TBD |
| rp2040 | ✅ | ✅ | ✅ | Arduino needs `raspberrypi` PIO pkg |
| stm32f103 | ✅ | ✅ | ✅ | |
| stm32f401 | ✅ | ✅ | ✅ | |
| stm32f407 | ✅ | ❌ | ✅ | Add Zephyr `nucleo_f407zg` if west board ok |
| stm32g474re | ✅ | ✅ | ✅ | |
| stm32h563 | ✅ | ✅ | ✅ | |
| stm32h735 | ❌ | ❌ | — | Chip model; framework path TBD |
| stm32l073 | ✅ | ✅ | ✅ | |
| stm32l476 | ✅ | ✅ | ✅ | |
| stm32wb55 | ✅ | ✅ | ✅ | |
| stm32wba52 | ✅ | ✅ | ✅ | Custom PIO board JSON |
| mkw41z4 | ❌ | ✅ | — | Zephyr only today |

Legend: ✅ in `validation/*-matrix/boards.yaml` · ❌ not yet · — blocked

## How peripherals attach

1. **System YAML** (`validation/arduino-matrix/systems/<board>.yaml`):
   `external_devices` lists kits (`type: ina219`, `connection: i2c0|i2c1`).
2. **Kit registry** (`crates/core/src/peripherals/kit/`) implements the device.
3. **Firmware** uses stock Arduino `Wire` / Zephyr drivers — no LabWired SDK.
4. **Oracle**: UART marker `LW_L3_OK` after successful I2C probe (+ optional
   inspect artifacts later).

L0–L2 systems **keep** the INA219 attached; unused devices do not break UART
oracles.

## Commands

```bash
# Arduino full depth (needs PlatformIO platforms)
cargo build -p labwired-cli --release
python3 validation/arduino-matrix/run_matrix.py
python3 validation/arduino-matrix/run_matrix.py --sketches L3_i2c_sensor
python3 validation/arduino-matrix/run_matrix.py --boards stm32f103,esp32c3 --sketches L0_serial_boot,L3_i2c_sensor

# Zephyr (needs west + ZEPHYRPROJECT)
python3 validation/zephyr-matrix/run_matrix.py
python3 validation/zephyr-matrix/run_matrix.py --boards stm32l476,nrf52840

# Coverage table (this doc + boards.yaml)
python3 validation/framework_fleet_report.py
```

## CI

| Job | Scope |
|-----|--------|
| `arduino-matrix-gate` | Thin fleet (see workflow) — expand toward all Arduino boards |
| Onboarding / coverage-matrix | Bare-metal UART fixtures (not framework stock) |
| Zephyr matrix | **Not yet in CI** (needs west image) |

## Roadmap

1. **Done (this change):** L3 I2C + INA219 on all Arduino matrix systems; fleet doc.
2. Expand CI Arduino matrix to **all** `boards.yaml` rows × L0+L2 (+ L3 pilot).
3. Zephyr L3 sensor sample + CI west container.
4. ESP Zephyr when survival path exists.
5. SPI/UART kits as L4 (display, radio) with board pin maps.
