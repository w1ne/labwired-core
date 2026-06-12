# ESP32-S3

The Espressif ESP32-S3 (dual-core Xtensa LX7, 512 KB SRAM, external QSPI
flash, Wi-Fi + BLE 5, USB) is LabWired's reference Xtensa target. All
declared peripherals are **declarative** — the chip yaml references
external YAML descriptors under `configs/peripherals/esp32s3/`.

There are two chip yamls for ESP32-S3 today:

- `configs/chips/esp32s3.yaml` — minimal declarative model (UART0 + GPIO +
  TIMG0 + INTERRUPT_CORE0) used by older blinky / hello-world examples.
- `configs/chips/esp32s3-zero.yaml` — fuller "Plan 2/4" model used by
  the [`esp32s3-i2c-tmp102`](../../examples/esp32s3-i2c-tmp102/) demo;
  adds I²C0, SYSTIMER, USB-Serial-JTAG, RTC_CNTL stub, eFuse stub,
  IRAM + ROM-thunks + flash-XIP mapping. **Use this variant for new
  ESP32-S3 work** — the simulator's `configure_xtensa_esp32s3` is the
  authoritative wiring code that this yaml documents.

## Status at a glance

| Aspect              | Status                                                                          |
|---------------------|---------------------------------------------------------------------------------|
| Chip yamls          | [`esp32s3.yaml`](../../configs/chips/esp32s3.yaml) (minimal) · [`esp32s3-zero.yaml`](../../configs/chips/esp32s3-zero.yaml) (Plan 2/4) |
| System yaml         | [`configs/systems/esp32s3-zero.yaml`](../../configs/systems/esp32s3-zero.yaml)  |
| Reference firmware  | [`examples/esp32s3-i2c-tmp102/`](../../examples/esp32s3-i2c-tmp102/) (Plan 4, canonical) · `examples/esp32s3-blinky/` · `examples/esp32s3-hello-world/` |
| Validation          | Plan 4 end-to-end test: `crates/core/tests/e2e_i2c_tmp102.rs`                   |
| Tier                | structural — declarative + Plan 2/4 peripheral models                           |

## Peripherals (esp32s3-zero, Plan 2/4 model)

| Peripheral       | Base       | Status              | Notes                                                                          |
|------------------|------------|---------------------|--------------------------------------------------------------------------------|
| Xtensa LX7       | —          | ✅ modeled (dual)   | PRO_CPU + APP_CPU round-robin (`Machine::with_secondary_cpu`); FPU coprocessor not modeled |
| IRAM             | 0x40370000 | ✅ ram region       | 512 KiB                                                                        |
| ROM thunks       | 0x40000000 | ✅ thunk bank       | 384 KiB internal ROM call sites                                                |
| Flash (I-cache)  | 0x42000000 | ✅ XIP mapping      | 4 MiB                                                                          |
| Flash (D-cache)  | 0x3C000000 | ✅ XIP mapping      | 4 MiB                                                                          |
| USB Serial/JTAG  | 0x60038000 | ✅ modeled          | 4 KiB — used as console in Plan 4 demo                                         |
| SYSTIMER         | 0x60023000 | ✅ modeled          | 4 KiB — periodic alarm in Plan 4 demo                                          |
| SYSTEM           | 0x600C0000 | ⚙ stub             | 4 KiB                                                                          |
| RTC_CNTL         | 0x60008000 | ⚙ stub             | 4 KiB                                                                          |
| eFuse            | 0x60007000 | ⚙ stub             | 4 KiB                                                                          |
| I²C0             | 0x60013000 | ✅ modeled          | 4 KiB — round-trip exercised in [`esp32s3-i2c-tmp102`](../../examples/esp32s3-i2c-tmp102/) |
| GDMA             | 0x6003F000 | ✅ modeled          | 5 channels; M2M descriptor walks **and** peripheral-coupled byte movement (see below) |
| SPI2 / SPI3      | 0x60024000 / 0x60025000 | ✅ modeled | GP-SPI master: CPU W-buffer path + GDMA-coupled DMA transactions; device attach surface (e-paper e2e) |
| I²S0 / I²S1      | 0x6000F000 / 0x6002D000 | ✅ modeled | Config/control twin + GDMA-coupled sample streaming (`RXEOF_NUM` byte semantics) |
| UART0/1/2        | 0x60000000 / 0x60010000 / 0x6002E000 | ✅ modeled | FIFO + STATUS + baud-paced drain; UART0 doubles as the UHCI0 DMA endpoint |

The authoritative wiring (`configure_xtensa_esp32s3`) additionally registers
LEDC, RMT, PCNT, TIMG0/1, SAR-ADC, TWAI, AES, SHA, RSA, HMAC, DS, MCPWM0/1,
SD/MMC, LCD_CAM and USB-OTG models — see
`crates/core/src/system/xtensa.rs` for the full list and fidelity notes.

The plain `esp32s3.yaml` (used by older examples) is a strict subset:
UART0 @ 0x60000000 + the declarative GPIO/TIMG0/INTERRUPT_CORE0 peripherals
under [`configs/peripherals/esp32s3/`](../../configs/peripherals/esp32s3/).
See [`docs/declarative_registers.md`](../declarative_registers.md) for
how declarative peripheral descriptors work.

## DMA (GDMA)

The GDMA model (`crates/core/src/peripherals/esp32s3/gdma.rs`) moves real
bytes:

- **Memory-to-memory** (`MEM_TRANS_EN`): full descriptor-chain walks with
  owner-bit writeback.
- **Peripheral-coupled mode** (routed by `IN_PERI_SEL` / `OUT_PERI_SEL`):
  UART (UHCI0), SPI2, SPI3, I²S0 and I²S1 transfer real bytes between
  descriptor chains and the peripheral models — incremental 64-byte-per-tick
  pumps, per-descriptor owner/length writeback, per-direction EOF
  interrupts (sources 66–75).
- Other peripheral ids (AES, SHA, ADC, RMT, LCD_CAM, unknown/unbound) keep
  an auto-complete fallback: EOF latches without byte movement so polling
  firmware progresses. LCD_CAM coupling is deferred by design.

## Not yet modeled (commonly expected on ESP32-S3)

Still absent from the model: **WIFI / BT controller**, **RTC_IO**,
**XTS_AES**, **TEMP_SENSOR**, **APB_CTRL**, **PSRAM cache controller**,
**FPU coprocessor**.

Firmware that touches unmapped registers will hit
`MemoryAccessViolation` or stall in a polling loop. See
[`docs/getting_started_firmware.md`](../getting_started_firmware.md).
