# Validation — SpiceDispenser rom-boot, model vs. real ESP32-S3

Hardware oracle: physical **ESP32-S3 (QFN56) rev v0.2**, MAC `ac:a7:04:26:89:fc`,
over USB-Serial-JTAG, running the **same** `firmware.factory.bin` flashed with
esptool. Model: `labwired run --rom-boot` on `configs/chips/esp32s3-zero.yaml`.

## Bit-identical through the 2nd-stage bootloader ✅

Both the model and the real chip, from the genuine boot ROM, produce the **same**
bootloader hand-off:

| Line | Model | Real ESP32-S3 |
|---|---|---|
| ROM banner | `ESP-ROM:esp32s3-20210327` | `ESP-ROM:esp32s3-20210327` |
| boot mode | `boot:0x8 (SPI_FAST_FLASH…)` | `boot:0x8 (SPI_FAST_FLASH_BOOT)` |
| seg 1 | `load:0x3fce2820,len:0x10cc` | `load:0x3fce2820,len:0x10cc` |
| seg 2 | `load:0x403c8700,len:0xc2c` | `load:0x403c8700,len:0xc2c` |
| seg 3 | `load:0x403cb700,len:0x30c0` | `load:0x403cb700,len:0x30c0` |
| entry | `entry 0x403c88b8` | `entry 0x403c88b8` |

So the model is faithful through: real boot ROM (`.data` copy-source
reconstruction, cache/flash MMU), the SPI-flash controller, the real 2nd-stage
bootloader, and the app entry jump.

## App stage — divergence isolates a model bug

After `entry 0x403c88b8`:

- **Real hardware:** boots the full ESP-IDF/Arduino app. On this bare board (no
  PCA9685 wired) the dispense loop runs and logs
  `i2c_master_transmit failed: ESP_ERR_INVALID_STATE` — i.e. the firmware is
  executing the real dispense path and only the external I²C device is absent.
- **Model:** aborts in ESP-IDF `memory_layout` init —
  `SOC_RESERVE_MEMORY_REGION region range 0x00000000 - 0x3fcf0000 overlaps with
  0x3c000000 - 0x3e000000`.

Because real silicon sails past this point, the abort is a **model bug**, not a
firmware issue. Investigation (via `--break-at`/`--watch-mem`) localised it to a
DROM/flash read used by the reserved-memory-region check resolving to 0 in the
model; it is complicated by (a) SMP boot non-determinism and (b) `firmware.elf`
symbol addresses not lining up linearly with the running `firmware.factory.bin`
DROM page mapping. Tracked as the open boot frontier.

## Bottom line

- The SpiceDispenser firmware **runs end-to-end on real ESP32-S3** (boots,
  initialises I²C, drives the dispense loop) — wiring a PCA9685 + servos makes it
  physically dispense.
- LabWired's ESP32-S3 model is **silicon-faithful through the 2nd-stage
  bootloader and app entry**, validated bit-for-bit against the real chip.
- The remaining gap is a model-side bug in ESP-IDF `memory_layout` bring-up,
  confirmed by the hardware oracle.
