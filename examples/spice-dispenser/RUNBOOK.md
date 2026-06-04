# Runbook — SpiceDispenser faithful rom-boot + hardware validation

## 1. Build the firmware flash image

From the SpiceDispenser repo (`firmware/`, the PlatformIO esp32-s3 env):

```
pio run -e esp32-s3
# produces .pio/build/esp32-s3/firmware.factory.bin  (merged bootloader+parts+app)
```

## 2. Generate the real ROM/DROM images (not vendored)

The ESP32-S3 boot ROM is Espressif-copyright; extract flat images from the copy
that ships with the ESP toolchain:

```
python3 core/scripts/make_esp32s3_rom_bins.py \
    ~/.platformio/tools/tool-esp-rom-elfs/esp32s3_rev0_rom.elf  /tmp
# writes /tmp/esp32s3_rom.bin (IROM, by load-address) and /tmp/esp32s3_drom.bin
```

The script lays the IROM image by **load address** and reconstructs the boot
ROM's `.data` copy sources, so the ROM's own startup initialises its pointers
(e.g. `rom_cache_internal_table_ptr`) — required for the cache routines to run.

## 3. Run the faithful rom-boot in LabWired

```
export LABWIRED_ESP32S3_ROM=/tmp/esp32s3_rom.bin
export LABWIRED_ESP32S3_DROM=/tmp/esp32s3_drom.bin
export LABWIRED_ESP32S3_FLASH=.../firmware/.pio/build/esp32-s3/firmware.factory.bin

cargo run --release -p labwired-cli -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware .../firmware/.pio/build/esp32-s3/firmware.elf \
    --rom-boot --max-steps 40000000
```

Expected: the real ROM banner, the 2nd-stage bootloader `load:`/`entry` lines,
then ESP-IDF app startup. Debugging hooks: `--break-at <pc>` (dump a0..a15 +
window state on first hit, both cores) and `--watch-mem <addr>` (dump a u32).

## 4. Validate on real hardware (the oracle)

With a physical ESP32-S3 on USB-Serial-JTAG (`/dev/ttyACM*`):

```
# flash the same factory image
python3 ~/.platformio/packages/tool-esptoolpy/esptool.py \
    --chip esp32s3 --port /dev/ttyACM0 --baud 460800 \
    write_flash --flash_mode dio --flash_size 4MB 0x0 \
    .../firmware/.pio/build/esp32-s3/firmware.factory.bin

# monitor (USB-Serial-JTAG re-enumerates on reset; reconnect across ports)
pio device monitor -p /dev/ttyACM0 -b 115200
```

The board's bootloader output must match the model's `load:`/`entry` lines
(it does — see VALIDATION.md). On a bare board (no PCA9685 wired) the app boots
fully and the dispense loop logs I²C errors (`ESP_ERR_INVALID_STATE`) because no
device ACKs on the bus — i.e. the firmware is running the real dispense path.
