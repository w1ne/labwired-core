# Vendored ESP32-S3 boot-ROM images

Flat mask-ROM images for the faithful `--rom-boot` path, embedded into
non-wasm builds so LabWired runs unmodified ESP32-S3 firmware out of the box
with no ESP toolchain installed.

| file | window | size |
|---|---|---|
| `esp32s3_rom.bin` | IROM (instruction bus) `0x4000_0000..0x4006_0000` | 384 KiB |
| `esp32s3_drom.bin` | DROM (data bus) `0x3FF0_0000..0x3FF2_0000` | 128 KiB |

## Provenance

Extracted from Espressif's published `esp32s3_rev0_rom.elf`
(distributed in [espressif/esp-rom-elfs](https://github.com/espressif/esp-rom-elfs)
and shipped with PlatformIO as `tool-esp-rom-elfs` and with ESP-IDF) by
`scripts/make_esp32s3_rom_bins.py`. The Rust extractor
(`crates/core/src/boot/esp32s3_rom.rs::extract_rom_images`) produces
byte-identical output from the same ELF.

The ROM contents are the ESP32-S3 mask ROM, copyright Espressif Systems
(Shanghai) Co., Ltd. They are included here solely so the simulator can
execute the chip's genuine boot path; they are not covered by this
repository's MIT license.

## Resolution order at runtime

1. `LABWIRED_ESP32S3_ROM` / `LABWIRED_ESP32S3_DROM` (explicit flat bins)
2. ROM ELF from an installed toolchain (`LABWIRED_ESP32S3_ROM_ELF`,
   PlatformIO, ESP-IDF), extracted and cached
3. These vendored images (non-wasm builds only)

`LABWIRED_ESP32S3_FASTBOOT=1` opts out of all of the above and forces the
fast-boot/harness path.
