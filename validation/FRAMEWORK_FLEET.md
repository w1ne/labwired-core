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
| **L4** | `L4_spi_sensor` | *(Zephyr planned)* | **SPI kit** on SPI / Zephyr `spi` (MAX31855 thermocouple) |
| **L5** | `L5_adc` | *(planned)* | **ADC** via `analogRead` |
| **L6** | `L6_pwm` | *(planned)* | **PWM** via `analogWrite` |
| **L7** | `L7_timer` | *(via L1 sleep)* | **Time base** — `micros()` advances (SysTick/RTC/TIM) |
| **L8** | `L8_can` | *(labs)* | **CAN loopback** (bxCAN/FDCAN registers; not portable Arduino API) |

## Chip coverage (record of intent)

| Chip | Arduino matrix | Zephyr matrix | L3 I2C kit | Notes |
|------|----------------|---------------|------------|-------|
| esp32 | ✅ L0–L4 | ❌ no ESP Zephyr path yet | ✅ Arduino | L4 SPI MISO from kits; FreeRTOS dual-core |
| esp32c3 | ✅ L0–L8 (9 pass / 0 skip) | ❌ | ✅ Arduino | L4 SPI green; RISC-V FreeRTOS; L8 TWAI self-RX on the shared S3 TWAI model (C3 source 25) |
| esp32s3 | ✅ L0–L4 | ❌ | ✅ Arduino | L4 via spi2_s3 Esp32s3Spi; dual-core |
| esp32c6 | ✅ L0–L8 (9/9) | ❌ | ✅ Arduino (INA219 on GPIO23/22) | Arduino-ESP32 3.x (IDF 5.5, CI pins espressif32@8.0.0) on the in-tree C6 mask ROM. L2 = RGB LED_BUILTIN through RMT TX (C3 model, source 49). L3 = `Wire.begin(23, 22)` (variant SDA/SCL) reads the INA219 config 0x399F + bus 0x19CA. L6 = `analogWrite(2, 128)`: 1 kHz / 50 % pad edges on GPIO2 (LEDC on the PCR XTAL clock). L5 = oneshot APB_SARADC: `analogRead(A0)` 256, `A1` 529 (REG_CLK_EN gate, PWDET stub). L8 = TWAI0 @0x6000B000 (S3/C3 TWAI model, source 46) self-RX of ID 0x123 / 0xA5 |
| nrf52832 | ✅ L0–L4 | ✅ | ✅ Arduino; Zephyr system+i2c0 | L4 legacy SPI; TWIM overlay Zephyr L3 |
| nrf52840 | ✅ L0–L4 | ✅ | ✅ both | L4 legacy SPI; Zephyr L3 green |
| nrf5340 | ❌ | ✅ | ❌ no I2C model | Add Arduino when PIO path exists |
| nrf54l15 | ❌ | ❌ | — | Chip model present; frameworks TBD |
| rp2040 | ✅ L0–L4 | ✅ | ✅ Arduino; Zephyr L3 skip | L4 SPI slave attach + bootrom auto-load |
| stm32f103 | ✅ L0–L8 (9 pass / 0 skip) | ✅ | ✅ both | F1 Wire + Zephyr poll green; L6 on PA0 TIM2_CH1, pad-proven 1.000 kHz / 50.2 % (PC13 has no timer channel) |
| stm32f401 | ✅ L0–L4 | ✅ | ✅ both | |
| stm32f407 | ✅ L0–L4 | ❌ | ✅ Arduino | Add Zephyr `nucleo_f407zg` if west board ok |
| stm32f411ceu6 | ✅ L0–L7 (L8 n/a, no CAN) | ❌ | ✅ Arduino (L3 ACK-only, F4 sketch ifdef) | WeAct Black Pill (`blackpill_f411ce`); L6 on PA0 TIM2_CH1, pad edges not routed |
| stm32g474re | ✅ L0–L4 | ✅ | ✅ Arduino | |
| stm32g071rb | ✅ L0–L7 (L8 n/a, no CAN) | ❌ | ✅ Arduino (L3 exact tier) | NUCLEO-G071RB (`nucleo_g071rb`); L6 on PA0 TIM2_CH1 (PA5 is DAC1_OUT2), pad edges not routed |
| stm32h563 | ✅ L0–L4 | ✅ | ✅ Arduino | |
| stm32h735 | ❌ | ❌ | — | Chip model; framework path TBD |
| stm32l073 | ✅ L0–L4 | ✅ | ✅ Arduino | |
| stm32l476 | ✅ L0–L4 | ✅ | ✅ Arduino | |
| stm32u575 | ✅ L0–L8 (9 pass / 0 skip) | ✅ L0–L3 | ✅ both | U5 RCC ready bits + CRS; Zephyr `nucleo_u575zi_q` |
| stm32u545 | ✅ L0–L8 | ❌ | ✅ Arduino | Generic `U545RETxQ` variant with NUCLEO-U545RE-Q pin overrides; Zephyr not validated |
| stm32g0b1re | ✅ L0–L8 (9 pass / 0 skip) | ❌ | ✅ Arduino | NUCLEO-G0B1RE; RCC `stm32g0b1` (HSI48), Serial = LPUART1, L6 on PA0 TIM2_CH1 (pad edges not routed), L8 FDCAN1 register-level loopback |
| stm32wb55 | ✅ L0–L4 | ✅ | ✅ Arduino | |
| stm32wba52 | ✅ L0–L4 | ✅ | ✅ Arduino | Custom PIO board JSON |
| atmega328p | ✅ L0–L7 (8 pass / 1 skip, no CAN) | ❌ | ✅ Arduino TWI | Nano: USART+PORT+Timer0/1/2+TWI+SPI on CPU; L6 on D9 OC1A, pad-proven 490 Hz phase-correct (D13 has no timer output) |
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
| `arduino-matrix-gate` | All Arduino boards × **L0–L4** (16×5; skipped cells ok) |
| Onboarding / coverage-matrix | Bare-metal UART fixtures (not framework stock) |
| Zephyr matrix | **Not yet in CI** (needs west image) — run locally |

## Roadmap

1. **Arduino L0–L4 green** on all 16 matrix boards (landed — see `docs/coverage/arduino-scoreboard.md`).
2. **Zephyr L3** sample + systems; prove F1/nRF/RP first.
3. Fix STM32 classic-SPI one-byte residual (L4 accepts `0x00019016` lag only).
4. Zephyr L4 MAX31855 + ESP Zephyr when survival path exists; Zephyr matrix in CI west container.
