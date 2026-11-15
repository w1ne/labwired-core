# ESP32 family: stock `Preferences` (NVS) now keeps what it writes

**Demand rank:** ESP32 is #1 (38 % of respondents, JetBrains State of Embedded
2025). This is depth work on parts already modelled, not a new part.
`Preferences` is the standard way an ESP32 Arduino sketch stores settings, so
every sketch that saves Wi-Fi credentials, calibration or a boot counter
depends on it.

**Vendor status** (espressif.com/en/products/longevity-commitment, read
2026-11-09): ESP32 "At Least 15 Years" from 2016; ESP32-S3 and ESP32-C3 "At
Least 12 Years" from 2021; ESP32-C6 "At Least 12 Years" from 2023. None are
marked NRND or EOL.

## What was wrong

Before this change the drivers reported success while the data was lost.
`putUInt` returned 4, but `getUInt` read back the default value.

| chip | flash controller in the engine | defect |
|---|---|---|
| ESP32 | `esp32_spi` (SPI1) | ESP-IDF programs and erases with **USR** commands (op `0x02` page program, op `0x20` sector erase). The model only clocked these out on the wire and never wrote the flash backing. The opcodes were declared `#[allow(dead_code)]`. |
| ESP32-C3 / S3 / C6 | `SpiMemFlash` (SPI_MEM1) | `spi_flash_hal_program_page` writes `ADDR = (addr & 0xFFFFFF) \| (len << 24)` and then sets `CMD.FLASH_PP` (checked in the disassembly of the S3 image). The model used the whole ADDR register as the address, so each write landed past the end of the array. |
| ESP32-C6 | fast boot | ROM `.data` is never unpacked. That left `g_rom_flashchip.chip_size` (`rom_default_spiflash_legacy_data`, `0x4087_FA08`) at 0, which made `esp_flash_default_chip.size` 0. Every `esp_flash_read` then returned `ESP_ERR_INVALID_ARG`, and `Preferences.begin()` failed. |

## Proof

`sketch/main.ino` is the firmware: stock arduino-esp32 3.3.x (PlatformIO
`espressif32@8.0.0`, framework 4.30312.261024) built for `esp32dev`,
`esp32-c3-devkitm-1`, `esp32-s3-devkitc-1` and `esp32-c6-devkitc-1`. It runs on
the Arduino-matrix systems (`validation/arduino-matrix/systems/<chip>.yaml`).
The sketch:

1. bumps a boot counter and reads it back,
2. stores and reads back a string,
3. overwrites one key 800 times. That fills the 5-page NVS partition several
   times, so NVS must reclaim pages with sector erases.
4. re-reads all values and removes a key.

| chip | before (`before/`) | after (`after/`) | erases (`after/erase-trace.txt`) |
|---|---|---|---|
| ESP32 | `readback=0 name= ctr=0`, FAIL | `LW_NVS_OK` | 3 × USR `0x20` @ 0xA000/0xB000/0xC000 |
| ESP32-C3 | `readback=0 name= ctr=0`, FAIL | `LW_NVS_OK` | 3 × dedicated SE |
| ESP32-S3 | `readback=0 name= ctr=0`, FAIL | `LW_NVS_OK` | 3 × dedicated SE |
| ESP32-C6 | `begin=0`, FAIL | `LW_NVS_OK` | 3 × dedicated SE |

"before" is `origin/main` at a5f5c409f. "after" is this branch. Both runs used
the same ELFs and test scripts (`max_steps` 400M, `wall_time_ms` 600000, marker
`LW_NVS_OK`).

## Not covered

- Persistence across a power cycle. Each run starts from a freshly erased
  flash backing, so the counter starts at 0.
- Flash encryption and `nvs_flash_secure_init`.
- The ESP32-S2 matrix row exists only on the lane branch, so S2 was not run
  here.
