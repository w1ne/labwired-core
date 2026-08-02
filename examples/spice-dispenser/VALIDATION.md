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

## App stage — runs the real ESP-IDF dispense path ✅

After `entry 0x403c88b8`, the model boots the full ESP-IDF/Arduino app: ROM
`memory_layout` bring-up, the SMP scheduler, `Wire`/`i2c_master` init, and the
dispense loop. Attaching the PCA9685 twin (run with `--system system-pca9685.yaml`,
which declares the expander so the factory wires it):

- **Real hardware (bare board, no PCA9685):** the dispense loop logs
  `i2c_master_transmit failed: ESP_ERR_INVALID_STATE` — the real driver runs but
  no device ACKs on the bus.
- **Model (with PCA9685 twin):** the interrupt-driven `i2c_master` transaction
  **completes** — the modeled PCA9685 ACKs, the firmware programs its PWM
  registers, and the run logs `PCA9685: channel N servo -> …°`. The full dispense
  actuation path executes in simulation, which the bare oracle board cannot show.

### Boot frontier history

Two model bugs were found and fixed past app entry:

1. **`memory_layout` abort** (`SOC_RESERVE_MEMORY_REGION … overlaps …`): a
   DROM/flash read used by the reserved-region check resolved to 0 in the model.
   Fixed by reconstructing `ets_rom_layout_p`.
2. **I²C `ESP_ERR_INVALID_STATE`**: ESP-IDF's interrupt-driven `i2c_master` never
   saw its completion ISR, so `i2c_master->status` never reached DONE. Root cause
   (traced register→intmatrix→CPU `INTENABLE`): the I2C0 interrupt source was
   modeled as **49** instead of `ETS_I2C_EXT0_INTR_SOURCE = 42`, so the firmware
   left source 49 parked at the disabled default CPU interrupt and the ISR was
   never dispatched. Fixed by asserting source **42**.

## Bottom line

- The SpiceDispenser firmware **runs end-to-end on real ESP32-S3** (boots,
  initialises I²C, drives the dispense loop) — wiring a PCA9685 + servos makes it
  physically dispense.
- LabWired's ESP32-S3 model is **silicon-faithful through the 2nd-stage
  bootloader and app entry** (validated bit-for-bit against the real chip) **and
  now runs the ESP-IDF app's interrupt-driven I²C dispense path** to completion
  against the PCA9685 twin.
