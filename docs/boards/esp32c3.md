# ESP32-C3

The Espressif ESP32-C3 (single-core RISC-V RV32IMC, 400 KB SRAM, external
QSPI flash, Wi-Fi + BLE) is LabWired's reference RISC-V Espressif target.
All declared peripherals are **declarative** — the chip yaml references
external YAML descriptors under `configs/peripherals/esp32c3/`.

## Status at a glance

| Aspect              | Status                                                                            |
|---------------------|-----------------------------------------------------------------------------------|
| Chip yaml           | [`configs/chips/esp32c3.yaml`](../../configs/chips/esp32c3.yaml)                  |
| System yaml         | [`configs/systems/esp32c3-devkit.yaml`](../../configs/systems/esp32c3-devkit.yaml) |
| Reference firmware  | `examples/esp32c3/` (see folder)                                                  |
| Validation          | No `VALIDATION.md` checked in                                                     |
| Tier                | structural — declarative peripheral models only                                   |

## Peripherals (from chip yaml)

| Peripheral       | Base       | Status              | Notes                                                                           |
|------------------|------------|---------------------|---------------------------------------------------------------------------------|
| RV32IMC core     | —          | ✅ modeled          | Single core, RISC-V                                                             |
| UART0            | 0x60000000 | ✅ modeled          | 512-byte window, RISC-V (not stm32v2) profile                                   |
| GPIO             | 0x60004000 | ⚙ declarative      | `configs/peripherals/esp32c3/gpio.yaml`                                         |
| TIMG0            | 0x6001F000 | ⚙ declarative      | `configs/peripherals/esp32c3/timg0.yaml`                                        |
| INTERRUPT_CORE0  | 0x600C2000 | ⚙ declarative      | `configs/peripherals/esp32c3/interrupt_core0.yaml`                              |
| ROM              | 0x40000000 | ⚙ declarative      | 384 KB internal ROM (`configs/peripherals/esp32c3/rom.yaml`)                    |

See [`docs/declarative_registers.md`](../declarative_registers.md) for
how declarative peripheral descriptors work.

## Not yet modeled (commonly expected on ESP32-C3)

The chip yaml does not declare: **WIFI / BT controller**, **RTC_CNTL**,
**RTC_IO**, **SYSTEM (clock + reset)**, **SPI0/1/2** (cache + general
purpose), **I²C0/1**, **UART1**, **LEDC**, **RMT**, **DMA**, **AES /
SHA / RSA / HMAC / DS / XTS_AES**, **ADC1/2**, **TEMP_SENSOR**, **USB
Serial/JTAG**, **TIMG1**, **SYSTIMER**, **GDMA**, **APB_CTRL**.

Firmware that touches any of these registers will hit
`MemoryAccessViolation` or stall in a polling loop. See
[`docs/getting_started_firmware.md`](../getting_started_firmware.md).
